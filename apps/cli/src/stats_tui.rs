//! Live resource-metrics TUI (`spky stats`).
//!
//! Consumes the project-level metrics SSE endpoint
//! (`GET /v1/projects/{pid}/metrics/stream?window=…`): the server backfills
//! the requested window, then tails new samples every ~15s. Each `data:` line
//! is one JSON point tagged with the app it came from (role + name), and the
//! TUI keeps one panel per app with four charts: CPU, memory, disk I/O and
//! network throughput.
//!
//! The stream thread auto-reconnects (idle reads are normal — samples land
//! every 15s — so the read timeout sits well above that), re-requesting a
//! window that covers the gap; the render thread dedupes by timestamp.

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph};
use ratatui::Terminal;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::{BufRead, IsTerminal};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Points kept per series — 24h at the 15s sample cadence.
const MAX_POINTS: usize = 5_760;
/// Redraw / input poll interval.
const TICK: Duration = Duration::from_millis(100);
/// Panel height in rows (title border + chart area).
const PANEL_HEIGHT: u16 = 9;

pub struct StatsArgs {
    pub base_url: String,
    pub auth_header: String,
    pub pid: String,
    pub slug: String,
    pub window: String,
    pub filter: Option<Vec<String>>,
}

/// One SSE sample, as emitted by the server (monitoring.RolePoint).
#[derive(Debug, Clone, Deserialize)]
struct RolePoint {
    role: String,
    #[serde(default)]
    name: String,
    ts: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    cpu_pct: f64,
    #[serde(default)]
    mem_bytes: i64,
    #[serde(default)]
    mem_limit_bytes: i64,
    #[serde(default)]
    disk_read_bps: i64,
    #[serde(default)]
    disk_write_bps: i64,
    #[serde(default)]
    net_rx_bps: i64,
    #[serde(default)]
    net_tx_bps: i64,
    /// E2E sync-pipeline latency, scheduler role only (the control plane
    /// scrapes it from the scheduler's `/metrics`). Absent on every other
    /// role, and on older control planes — hence `default`, and hence the
    /// "only render the chart if the series has data" rule in `draw_panel`.
    #[serde(default)]
    heartbeat_e2e_ms: i64,
    #[serde(default)]
    heartbeat_fails: i64,
}

enum Msg {
    Point(Box<RolePoint>),
    Status(String),
    Fatal(String),
}

/// Chart series for one app. Vecs of (unix_seconds, value) so they can be
/// handed to ratatui Datasets without copying.
#[derive(Default)]
struct RoleSeries {
    cpu: Vec<(f64, f64)>,
    mem: Vec<(f64, f64)>,
    disk_read: Vec<(f64, f64)>,
    disk_write: Vec<(f64, f64)>,
    net_rx: Vec<(f64, f64)>,
    net_tx: Vec<(f64, f64)>,
    /// E2E heartbeat latency. Trimmed on its own length, not with the block
    /// below: points only carry it for the scheduler, so it is shorter than
    /// the resource series and a shared cut index would eat live data.
    hb: Vec<(f64, f64)>,
    hb_fails: i64,
    mem_limit: f64,
    last_ts: f64,
}

impl RoleSeries {
    fn push(&mut self, p: &RolePoint) {
        let ts = p.ts.timestamp() as f64;
        if ts <= self.last_ts {
            return; // reconnect overlap
        }
        self.last_ts = ts;
        self.cpu.push((ts, p.cpu_pct));
        self.mem.push((ts, p.mem_bytes as f64));
        self.disk_read.push((ts, p.disk_read_bps as f64));
        self.disk_write.push((ts, p.disk_write_bps as f64));
        self.net_rx.push((ts, p.net_rx_bps as f64));
        self.net_tx.push((ts, p.net_tx_bps as f64));
        if p.mem_limit_bytes > 0 {
            self.mem_limit = p.mem_limit_bytes as f64;
        }
        if p.heartbeat_e2e_ms > 0 {
            self.hb.push((ts, p.heartbeat_e2e_ms as f64));
            if self.hb.len() > MAX_POINTS {
                let cut = self.hb.len() - MAX_POINTS;
                self.hb.drain(..cut);
            }
        }
        self.hb_fails = p.heartbeat_fails;
        if self.cpu.len() > MAX_POINTS {
            let cut = self.cpu.len() - MAX_POINTS;
            for v in [
                &mut self.cpu,
                &mut self.mem,
                &mut self.disk_read,
                &mut self.disk_write,
                &mut self.net_rx,
                &mut self.net_tx,
            ] {
                v.drain(..cut);
            }
        }
    }
}

/// Preferred panel order: infra first, user apps after.
fn role_rank(role: &str) -> usize {
    match role {
        "surrealdb" => 0,
        "scheduler" => 1,
        "ssp" => 2,
        "backend" => 3,
        "frontend" => 4,
        _ => 5,
    }
}

pub fn run(args: StatsArgs) -> Result<()> {
    if !std::io::stdout().is_terminal() {
        return run_ndjson(&args);
    }

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(std::io::stderr(), LeaveAlternateScreen);
        original_hook(info);
    }));

    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen, crossterm::cursor::Hide)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let result = run_app(&mut terminal, &args);

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::cursor::Show
    )?;

    result
}

fn run_app<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>, args: &StatsArgs) -> Result<()> {
    let (tx, rx): (Sender<Msg>, Receiver<Msg>) = std::sync::mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    // Unix seconds of the newest applied point; the stream thread reads it to
    // size the reconnect window so gaps are re-fetched, not lost.
    let newest_ts = Arc::new(AtomicU64::new(0));

    spawn_stream(args, tx, cancel.clone(), newest_ts.clone());

    let mut panels: BTreeMap<(usize, String), RoleSeries> = BTreeMap::new();
    let mut status = String::from("connecting…");
    let mut scroll: usize = 0;
    let filter = args.filter.as_ref().map(|f| {
        f.iter().map(|s| s.to_lowercase()).collect::<Vec<_>>()
    });

    loop {
        // Drain pending samples before each frame.
        loop {
            match rx.try_recv() {
                Ok(Msg::Point(p)) => {
                    if let Some(ref f) = filter {
                        let role = p.role.to_lowercase();
                        let name = p.name.to_lowercase();
                        if !f.iter().any(|x| *x == role || *x == name) {
                            continue;
                        }
                    }
                    let label = if p.name.is_empty() {
                        p.role.clone()
                    } else {
                        format!("{}/{}", p.role, p.name)
                    };
                    let key = (role_rank(&p.role), label);
                    let series = panels.entry(key).or_default();
                    series.push(&p);
                    let ts = p.ts.timestamp().max(0) as u64;
                    if ts > newest_ts.load(Ordering::Relaxed) {
                        newest_ts.store(ts, Ordering::Relaxed);
                    }
                    status = String::from("live");
                }
                Ok(Msg::Status(s)) => status = s,
                Ok(Msg::Fatal(e)) => {
                    cancel.store(true, Ordering::Relaxed);
                    anyhow::bail!(e);
                }
                Err(_) => break,
            }
        }

        terminal.draw(|f| draw(f, args, &panels, &status, scroll))?;

        if event::poll(TICK)? {
            if let Event::Key(key) = event::read()? {
                if handle_key(key, &mut scroll, panels.len()) {
                    cancel.store(true, Ordering::Relaxed);
                    return Ok(());
                }
            }
        }
    }
}

fn handle_key(key: KeyEvent, scroll: &mut usize, panel_count: usize) -> bool {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,
        KeyCode::Down | KeyCode::Char('j') => {
            *scroll = (*scroll + 1).min(panel_count.saturating_sub(1));
        }
        KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
        _ => {}
    }
    false
}

fn draw(
    f: &mut ratatui::Frame,
    args: &StatsArgs,
    panels: &BTreeMap<(usize, String), RoleSeries>,
    status: &str,
    scroll: usize,
) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    let header = Line::from(vec![
        Span::styled(
            format!(" {} ", args.slug),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("· window {} · ", args.window)),
        Span::styled(
            status.to_string(),
            Style::default().fg(if status == "live" {
                Color::Green
            } else {
                Color::Yellow
            }),
        ),
        Span::styled(
            "  (q quit, ↑/↓ scroll)",
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    f.render_widget(Paragraph::new(header), chunks[0]);

    let body = chunks[1];
    if panels.is_empty() {
        let msg = Paragraph::new(
            "waiting for samples… (new deployments need one 15s interval before the first point;\n free-plan projects have no resource metrics)",
        )
        .style(Style::default().fg(Color::DarkGray));
        f.render_widget(msg, body);
        return;
    }

    let visible = (body.height / PANEL_HEIGHT).max(1) as usize;
    let scroll = scroll.min(panels.len().saturating_sub(1));
    let mut y = body.y;
    for (i, ((_, label), series)) in panels.iter().enumerate() {
        if i < scroll || (i - scroll) >= visible {
            continue;
        }
        let h = PANEL_HEIGHT.min(body.y + body.height - y);
        if h < 4 {
            break;
        }
        draw_panel(f, Rect::new(body.x, y, body.width, h), label, series);
        y += h;
    }
}

fn draw_panel(f: &mut ratatui::Frame, area: Rect, label: &str, s: &RoleSeries) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", label));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // The scheduler carries a 5th series (e2e sync latency); every other role
    // keeps the 4-column layout rather than rendering an empty chart.
    let has_hb = !s.hb.is_empty();
    let cols = if has_hb {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Ratio(1, 5),
                Constraint::Ratio(1, 5),
                Constraint::Ratio(1, 5),
                Constraint::Ratio(1, 5),
                Constraint::Ratio(1, 5),
            ])
            .split(inner)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Ratio(1, 4),
                Constraint::Ratio(1, 4),
                Constraint::Ratio(1, 4),
                Constraint::Ratio(1, 4),
            ])
            .split(inner)
    };

    let cpu_now = s.cpu.last().map(|p| p.1).unwrap_or(0.0);
    render_chart(
        f,
        cols[0],
        &format!("cpu {:.0}%", cpu_now),
        &[(&s.cpu, Color::Cyan)],
        Unit::Percent,
    );
    let mem_now = s.mem.last().map(|p| p.1).unwrap_or(0.0);
    render_chart(
        f,
        cols[1],
        &format!(
            "mem {}{}",
            fmt_bytes(mem_now),
            if s.mem_limit > 0.0 {
                format!("/{}", fmt_bytes(s.mem_limit))
            } else {
                String::new()
            }
        ),
        &[(&s.mem, Color::Magenta)],
        Unit::Bytes,
    );
    let dr = s.disk_read.last().map(|p| p.1).unwrap_or(0.0);
    let dw = s.disk_write.last().map(|p| p.1).unwrap_or(0.0);
    render_chart(
        f,
        cols[2],
        &format!("disk r{} w{}", fmt_rate(dr), fmt_rate(dw)),
        &[(&s.disk_read, Color::Green), (&s.disk_write, Color::Red)],
        Unit::Rate,
    );
    let rx = s.net_rx.last().map(|p| p.1).unwrap_or(0.0);
    let tx_ = s.net_tx.last().map(|p| p.1).unwrap_or(0.0);
    render_chart(
        f,
        cols[3],
        &format!("net ↓{} ↑{}", fmt_rate(rx), fmt_rate(tx_)),
        &[(&s.net_rx, Color::Green), (&s.net_tx, Color::Blue)],
        Unit::Rate,
    );

    if has_hb {
        let hb_now = s.hb.last().map(|p| p.1).unwrap_or(0.0);
        // Consecutive failures are the tell that the number on screen is
        // stale: the last successful probe stays plotted while the pipeline
        // is already broken.
        let title = if s.hb_fails > 0 {
            format!("e2e {} !{}", fmt_ms(hb_now), s.hb_fails)
        } else {
            format!("e2e {}", fmt_ms(hb_now))
        };
        let color = if s.hb_fails > 0 { Color::Red } else { Color::Yellow };
        render_chart(f, cols[4], &title, &[(&s.hb, color)], Unit::Millis);
    }
}

enum Unit {
    Percent,
    Bytes,
    Rate,
    Millis,
}

fn render_chart(
    f: &mut ratatui::Frame,
    area: Rect,
    title: &str,
    series: &[(&Vec<(f64, f64)>, Color)],
    unit: Unit,
) {
    let (mut x_min, mut x_max) = (f64::MAX, f64::MIN);
    let mut y_max: f64 = 0.0;
    for (data, _) in series {
        for (x, v) in data.iter() {
            x_min = x_min.min(*x);
            x_max = x_max.max(*x);
            y_max = y_max.max(*v);
        }
    }
    if x_min > x_max {
        (x_min, x_max) = (0.0, 1.0);
    }
    if x_max - x_min < 1.0 {
        x_max = x_min + 1.0;
    }
    let y_top = match unit {
        Unit::Percent => (y_max * 1.15).max(100.0),
        _ => (y_max * 1.15).max(1.0),
    };
    let y_label = match unit {
        Unit::Percent => format!("{:.0}%", y_top),
        Unit::Bytes => fmt_bytes(y_top),
        Unit::Rate => fmt_rate(y_top),
        Unit::Millis => fmt_ms(y_top),
    };

    let datasets: Vec<Dataset> = series
        .iter()
        .map(|(data, color)| {
            Dataset::default()
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(*color))
                .data(data.as_slice())
        })
        .collect();

    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .title(Span::styled(title, Style::default().fg(Color::Gray)))
                .borders(Borders::NONE),
        )
        .x_axis(Axis::default().bounds([x_min, x_max]))
        .y_axis(
            Axis::default().bounds([0.0, y_top]).labels(vec![
                Span::styled("0", Style::default().fg(Color::DarkGray)),
                Span::styled(y_label, Style::default().fg(Color::DarkGray)),
            ]),
        );
    f.render_widget(chart, area);
}

fn fmt_bytes(v: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = v;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if v >= 100.0 || i == 0 {
        format!("{:.0}{}", v, UNITS[i])
    } else {
        format!("{:.1}{}", v, UNITS[i])
    }
}

fn fmt_rate(v: f64) -> String {
    format!("{}/s", fmt_bytes(v))
}

/// Latency label. Sub-second values are what a healthy pipeline produces, so
/// they stay in ms; anything past a second reads better as seconds.
fn fmt_ms(v: f64) -> String {
    if v >= 1000.0 {
        format!("{:.1}s", v / 1000.0)
    } else {
        format!("{:.0}ms", v)
    }
}

// ---------- SSE stream thread ----------

fn stream_url(args: &StatsArgs, window: &str) -> String {
    format!(
        "{}/v1/projects/{}/metrics/stream?window={}",
        args.base_url, args.pid, window
    )
}

fn spawn_stream(
    args: &StatsArgs,
    tx: Sender<Msg>,
    cancel: Arc<AtomicBool>,
    newest_ts: Arc<AtomicU64>,
) {
    let base = StatsArgs {
        base_url: args.base_url.clone(),
        auth_header: args.auth_header.clone(),
        pid: args.pid.clone(),
        slug: args.slug.clone(),
        window: args.window.clone(),
        filter: None,
    };
    thread::spawn(move || {
        let mut first = true;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            // First connect backfills the full requested window; reconnects
            // only need to cover the gap since the newest applied point.
            let window = if first {
                base.window.clone()
            } else {
                let newest = newest_ts.load(Ordering::Relaxed);
                let gap = if newest == 0 {
                    15 * 60
                } else {
                    (chrono::Utc::now().timestamp() as u64).saturating_sub(newest) + 60
                };
                format!("{}s", gap.clamp(60, 24 * 3600))
            };

            match stream_once(&stream_url(&base, &window), &base.auth_header, &tx, &cancel) {
                Ok(()) => return, // cancelled
                Err(e) => {
                    if first && e.starts_with("HTTP 4") {
                        // Auth/plan/404 errors won't heal by retrying.
                        let _ = tx.send(Msg::Fatal(e));
                        return;
                    }
                    let _ = tx.send(Msg::Status(format!("reconnecting… ({})", e)));
                }
            }
            first = false;
            thread::sleep(Duration::from_secs(2));
        }
    });
}

/// Read one SSE connection until error/EOF. Returns Ok(()) only on cancel.
fn stream_once(
    url: &str,
    auth: &str,
    tx: &Sender<Msg>,
    cancel: &Arc<AtomicBool>,
) -> std::result::Result<(), String> {
    // Read timeout well above the 15s sample cadence: an idle read that long
    // means the connection is actually dead, not just quiet.
    let agent = ureq::builder()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(45))
        .build();
    let resp = agent
        .get(url)
        .set("Authorization", auth)
        .set("Accept", "text/event-stream")
        .call();
    let resp = match resp {
        Ok(r) => r,
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            return Err(format!("HTTP {}: {}", code, body.trim()));
        }
        Err(ureq::Error::Transport(t)) => return Err(format!("connection error: {}", t)),
    };

    let reader = std::io::BufReader::with_capacity(512, resp.into_reader());
    let mut deadline = Instant::now();
    for line in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let line = line.map_err(|e| format!("read error: {}", e))?;
        if let Some(data) = line
            .strip_prefix("data: ")
            .or_else(|| line.strip_prefix("data:"))
        {
            // Server error events carry a bare JSON string ("query failed"),
            // which fails RolePoint parsing and is skipped here by design.
            if let Ok(p) = serde_json::from_str::<RolePoint>(data) {
                deadline = Instant::now();
                if tx.send(Msg::Point(Box::new(p))).is_err() {
                    return Ok(());
                }
            }
        }
        // Safety valve: a stream that yields lines but no parsable points for
        // 10 minutes is wedged; force a reconnect.
        if deadline.elapsed() > Duration::from_secs(600) {
            return Err("no samples for 10m".into());
        }
    }
    Err("stream closed".into())
}

// ---------- non-TTY fallback ----------

/// Pipe mode: emit raw NDJSON points to stdout (backfill, then live).
fn run_ndjson(args: &StatsArgs) -> Result<()> {
    let agent = ureq::builder()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(45))
        .build();
    let resp = agent
        .get(&stream_url(args, &args.window))
        .set("Authorization", &args.auth_header)
        .set("Accept", "text/event-stream")
        .call()
        .map_err(|e| anyhow::anyhow!("metrics stream: {}", e))?;
    let reader = std::io::BufReader::new(resp.into_reader());
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break, // idle timeout / server closed — history is out
        };
        if let Some(data) = line
            .strip_prefix("data: ")
            .or_else(|| line.strip_prefix("data:"))
        {
            println!("{}", data);
        }
    }
    Ok(())
}
