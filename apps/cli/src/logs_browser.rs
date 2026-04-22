//! Interactive log browser TUI (`spky cloud logs --interactive`).
//!
//! Layered on top of the same `GET /v1/projects/{pid}/logs` SSE endpoint the
//! live-tail path uses. Historical windows use `tail=false` so the server
//! closes the stream when the window is exhausted; follow-mode opens a second
//! stream with `tail=true` and appends entries as they arrive.
//!
//! Scroll-back pagination (fetch *older* windows on demand when the user
//! scrolls near the top) is implemented but kept deliberately conservative:
//! one in-flight fetch at a time, bounded ring buffer at 20k entries. Each
//! chunk is prepended on completion and the user's scroll position is
//! preserved by tracking the currently-focused entry index across buffer
//! growth.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Terminal;
use std::collections::VecDeque;
use std::io::BufRead;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::cloud::LogEntry;

/// How many entries the ring buffer holds before the oldest are dropped.
const MAX_BUFFERED: usize = 20_000;
/// When scrolling, how many entries from the top trigger an older-window fetch.
const SCROLL_BACK_THRESHOLD: usize = 200;
/// How many entries each older-window fetch asks for.
const SCROLL_BACK_CHUNK: usize = 5_000;
/// Redraw / input poll interval.
const TICK: Duration = Duration::from_millis(100);

pub struct LogsBrowserArgs {
    pub base_url: String,
    pub auth_header: String,
    pub pid: String,
    pub services: Option<Vec<String>>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub grep: Option<String>,
    pub follow: bool,
}

pub fn run(args: LogsBrowserArgs) -> Result<()> {
    // ---------- set up terminal ----------
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

    let result = run_app(&mut terminal, args);

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::cursor::Show
    )?;

    result
}

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    args: LogsBrowserArgs,
) -> Result<()> {
    let initial_since = args
        .since
        .unwrap_or_else(|| Utc::now() - chrono::Duration::hours(1));
    let initial_until = args.until;

    let mut app = App::new(AppState {
        base_url: args.base_url,
        auth_header: args.auth_header,
        pid: args.pid,
        services: args.services,
        grep: args.grep,
        window_since: initial_since,
        window_until: initial_until,
        follow_mode: args.follow,
    });

    app.start_initial_fetch();
    if app.state.follow_mode {
        app.start_follow_stream();
    }

    loop {
        app.drain_fetchers();
        terminal.draw(|f| app.render(f.area(), f))?;

        if event::poll(TICK)? {
            if let Event::Key(key) = event::read()? {
                if app.handle_key(key)? {
                    break;
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

struct AppState {
    base_url: String,
    auth_header: String,
    pid: String,
    services: Option<Vec<String>>,
    grep: Option<String>,
    window_since: DateTime<Utc>,
    window_until: Option<DateTime<Utc>>,
    follow_mode: bool,
}

struct App {
    state: AppState,

    /// Newest-last ring buffer of everything currently visible.
    entries: VecDeque<LogEntry>,
    /// Virtualised scroll position. Points into `entries`.
    list_state: ListState,

    /// Open text-entry mode for a popup.
    input_mode: InputMode,
    input_buffer: String,

    /// Client-side search regex. Matches highlighted in the main pane;
    /// `n` / `N` jump to next / previous match.
    search_regex: Option<regex::Regex>,
    /// When the user typed a bad regex, show it in the status bar instead
    /// of silently treating `/` as a no-op.
    search_error: Option<String>,

    /// In-flight fetcher(s). Chunks arrive on `rx` as `FetchMsg::*`.
    rx: Receiver<FetchMsg>,
    tx: Sender<FetchMsg>,

    /// Cancel flags for running threads. Dropping the Arc signals the thread
    /// to exit on the next SSE line; the thread does its own polling via
    /// `!cancel.load()`.
    follow_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,

    /// True while an initial / older-window fetch is in flight.
    loading_history: bool,
    /// True while a follow stream is active.
    following: bool,

    /// Oldest timestamp we've already asked for, so we know what to ask for
    /// next on scroll-back.
    oldest_loaded: Option<DateTime<Utc>>,
    /// Newest timestamp we've already loaded, so follow-mode knows where to
    /// resume from.
    newest_loaded: Option<DateTime<Utc>>,

    /// Last error from the fetcher, surfaced in the status bar.
    last_error: Option<String>,

    /// Set once an older-window fetch returns an empty response. Prevents
    /// the TUI from re-firing the same fetch in a loop every time the
    /// cursor lingers near the top of the buffer.
    no_more_older: bool,
}

enum InputMode {
    Normal,
    Search,
}

enum FetchMsg {
    Older { entries: Vec<LogEntry> },
    Appended { entries: Vec<LogEntry> },
    Error(String),
    HistoryDone,
    FollowClosed,
}

impl App {
    fn new(state: AppState) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self {
            state,
            entries: VecDeque::new(),
            list_state: ListState::default(),
            input_mode: InputMode::Normal,
            input_buffer: String::new(),
            search_regex: None,
            search_error: None,
            rx,
            tx,
            follow_cancel: None,
            loading_history: false,
            following: false,
            oldest_loaded: None,
            newest_loaded: None,
            last_error: None,
            no_more_older: false,
        }
    }

    // ---- fetching ----

    fn start_initial_fetch(&mut self) {
        self.loading_history = true;
        spawn_history_fetch(
            &self.state,
            self.state.window_since,
            self.state.window_until,
            self.tx.clone(),
            FetchKind::Initial,
        );
    }

    fn start_scroll_back_fetch(&mut self) {
        if self.loading_history || self.no_more_older {
            return;
        }
        let Some(current_oldest) = self.oldest_loaded else { return; };
        // Ask for the hour before the oldest line we currently have. If the
        // server returns nothing, we just stop paginating older.
        let next_since = current_oldest - chrono::Duration::hours(1);
        self.loading_history = true;
        spawn_history_fetch(
            &self.state,
            next_since,
            Some(current_oldest),
            self.tx.clone(),
            FetchKind::Older,
        );
    }

    fn start_follow_stream(&mut self) {
        if self.following {
            return;
        }
        let resume_from = self
            .newest_loaded
            .unwrap_or_else(|| Utc::now() - chrono::Duration::seconds(1));
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.follow_cancel = Some(cancel.clone());
        self.following = true;
        spawn_follow_stream(&self.state, resume_from, self.tx.clone(), cancel);
    }

    fn stop_follow_stream(&mut self) {
        if let Some(cancel) = self.follow_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.following = false;
    }

    fn drain_fetchers(&mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(FetchMsg::Older { mut entries }) => {
                    if entries.is_empty() {
                        // The backend has nothing in the requested hour —
                        // we've hit the retention floor or the server ran
                        // out of data. Stop firing scroll-back fetches.
                        self.no_more_older = true;
                        continue;
                    }
                    // Entries arrive oldest-first. We prepend them while
                    // shifting BOTH `selected` and `offset` by the grew-by
                    // amount, so the user's view stays visually anchored
                    // to the same entry while older lines pile up above.
                    let selected = self.list_state.selected();
                    let current_offset = self.list_state.offset();
                    entries.sort_by_key(|e| e.ts().unwrap_or_else(Utc::now));
                    if let Some(first) = entries.first().and_then(|e| e.ts()) {
                        self.oldest_loaded = Some(
                            self.oldest_loaded
                                .map(|o| o.min(first))
                                .unwrap_or(first),
                        );
                    }
                    if let Some(last) = entries.last().and_then(|e| e.ts()) {
                        self.newest_loaded = Some(
                            self.newest_loaded
                                .map(|n| n.max(last))
                                .unwrap_or(last),
                        );
                    }
                    let grew_by = entries.len();
                    for e in entries.into_iter().rev() {
                        self.entries.push_front(e);
                    }
                    // Trim the BACK (newest) when we overflow the ring, so
                    // a user who explicitly scrolled back doesn't lose the
                    // older context they just asked for.
                    while self.entries.len() > MAX_BUFFERED {
                        self.entries.pop_back();
                    }
                    let len = self.entries.len();
                    if let Some(s) = selected {
                        let new_sel = (s + grew_by).min(len.saturating_sub(1));
                        self.list_state.select(Some(new_sel));
                    }
                    let new_offset = (current_offset + grew_by).min(len.saturating_sub(1));
                    *self.list_state.offset_mut() = new_offset;
                }
                Ok(FetchMsg::Appended { entries }) => {
                    let was_at_bottom = self.is_at_bottom();
                    for e in entries {
                        if let Some(ts) = e.ts() {
                            self.newest_loaded = Some(
                                self.newest_loaded.map(|n| n.max(ts)).unwrap_or(ts),
                            );
                            if self.oldest_loaded.is_none() {
                                self.oldest_loaded = Some(ts);
                            }
                        }
                        self.entries.push_back(e);
                    }
                    self.trim_ring();
                    if self.state.follow_mode && was_at_bottom && !self.entries.is_empty() {
                        self.list_state.select(Some(self.entries.len() - 1));
                    }
                }
                Ok(FetchMsg::HistoryDone) => {
                    self.loading_history = false;
                    // Land the cursor at the newest line on first load.
                    if self.list_state.selected().is_none() && !self.entries.is_empty() {
                        self.list_state.select(Some(self.entries.len() - 1));
                    }
                }
                Ok(FetchMsg::FollowClosed) => {
                    self.following = false;
                    self.follow_cancel = None;
                }
                Ok(FetchMsg::Error(e)) => {
                    self.last_error = Some(e);
                    self.loading_history = false;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    fn trim_ring(&mut self) {
        while self.entries.len() > MAX_BUFFERED {
            self.entries.pop_front();
        }
    }

    fn is_at_bottom(&self) -> bool {
        match self.list_state.selected() {
            None => true,
            Some(s) => self.entries.is_empty() || s + 1 >= self.entries.len(),
        }
    }

    // ---- input ----

    /// Returns true to quit.
    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        match self.input_mode {
            InputMode::Normal => self.handle_normal_key(key),
            InputMode::Search => {
                self.handle_search_key(key);
                Ok(false)
            }
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(true);
            }

            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::PageDown | KeyCode::Char('d')
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    || key.code == KeyCode::PageDown =>
            {
                self.move_selection(20);
            }
            KeyCode::PageUp | KeyCode::Char('u')
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    || key.code == KeyCode::PageUp =>
            {
                self.move_selection(-20);
            }
            KeyCode::Char('g') => {
                if !self.entries.is_empty() {
                    self.list_state.select(Some(0));
                    self.maybe_paginate_older();
                }
            }
            KeyCode::Char('G') | KeyCode::End => {
                if !self.entries.is_empty() {
                    self.list_state.select(Some(self.entries.len() - 1));
                }
            }
            KeyCode::Home => {
                if !self.entries.is_empty() {
                    self.list_state.select(Some(0));
                    self.maybe_paginate_older();
                }
            }

            KeyCode::Char('/') => {
                self.input_mode = InputMode::Search;
                self.input_buffer.clear();
                self.search_error = None;
            }
            KeyCode::Char('n') => self.jump_to_match(1),
            KeyCode::Char('N') => self.jump_to_match(-1),

            KeyCode::Char('f') => {
                self.state.follow_mode = !self.state.follow_mode;
                if self.state.follow_mode {
                    self.start_follow_stream();
                } else {
                    self.stop_follow_stream();
                }
            }

            _ => {}
        }
        Ok(false)
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.input_buffer.clear();
            }
            KeyCode::Enter => {
                if self.input_buffer.is_empty() {
                    self.search_regex = None;
                    self.search_error = None;
                } else {
                    match regex::Regex::new(&self.input_buffer) {
                        Ok(re) => {
                            self.search_regex = Some(re);
                            self.search_error = None;
                            // Jump to first match at/after current selection.
                            self.jump_to_match(1);
                        }
                        Err(e) => {
                            self.search_regex = None;
                            self.search_error = Some(format!("bad regex: {}", e));
                        }
                    }
                }
                self.input_mode = InputMode::Normal;
                self.input_buffer.clear();
            }
            KeyCode::Backspace => {
                self.input_buffer.pop();
            }
            KeyCode::Char(c) => {
                self.input_buffer.push(c);
            }
            _ => {}
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0) as isize;
        let new = (current + delta).clamp(0, self.entries.len() as isize - 1) as usize;
        self.list_state.select(Some(new));
        // Moving the cursor up manually cancels follow-auto-scroll — matches lnav.
        if delta < 0 && self.state.follow_mode {
            // We still keep the stream open; we just stop auto-scrolling.
        }
        self.maybe_paginate_older();
    }

    fn maybe_paginate_older(&mut self) {
        if self.no_more_older {
            return;
        }
        let Some(selected) = self.list_state.selected() else { return; };
        if selected < SCROLL_BACK_THRESHOLD {
            self.start_scroll_back_fetch();
        }
    }

    fn jump_to_match(&mut self, direction: isize) {
        let Some(re) = self.search_regex.clone() else { return; };
        if self.entries.is_empty() {
            return;
        }
        let start = self.list_state.selected().unwrap_or(0) as isize;
        let len = self.entries.len() as isize;
        let mut i = start + direction;
        while (0..len).contains(&i) {
            if re.is_match(&self.entries[i as usize].message) {
                self.list_state.select(Some(i as usize));
                return;
            }
            i += direction;
        }
    }

    // ---- render ----

    fn render(&mut self, area: Rect, frame: &mut ratatui::Frame) {
        // Defensive clamping: background fetches and trims can leave both
        // `selected` and the internal `offset` pointing past the end of the
        // buffer. When that happens the List widget renders nothing and the
        // user sees a blank pane ("everything disappears at top/bottom").
        // Clamp both before handing the state to ratatui.
        let len = self.entries.len();
        if let Some(sel) = self.list_state.selected() {
            if len == 0 {
                self.list_state.select(None);
            } else if sel >= len {
                self.list_state.select(Some(len - 1));
            }
        }
        if len == 0 {
            *self.list_state.offset_mut() = 0;
        } else if self.list_state.offset() >= len {
            *self.list_state.offset_mut() = len.saturating_sub(1);
        }

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                // 1 row for content + 1 row for the bottom-border separator.
                Constraint::Length(2), // filter bar
                Constraint::Min(3),    // log list
                Constraint::Length(1), // input / help
                Constraint::Length(1), // status
            ])
            .split(area);

        self.render_filter_bar(layout[0], frame);
        self.render_list(layout[1], frame);
        self.render_input_or_help(layout[2], frame);
        self.render_status(layout[3], frame);
    }

    fn render_filter_bar(&self, area: Rect, frame: &mut ratatui::Frame) {
        let services = match &self.state.services {
            Some(svc) if !svc.is_empty() => svc.join(","),
            _ => "all".to_string(),
        };
        let range = match self.state.window_until {
            Some(u) => format!(
                "{} → {}",
                short_ts(self.state.window_since),
                short_ts(u)
            ),
            None => format!("{} → now", short_ts(self.state.window_since)),
        };
        let search = match (&self.search_regex, &self.search_error) {
            (_, Some(err)) => format!("err:{}", err),
            (Some(re), _) => format!("/{}/", re.as_str()),
            (None, _) => "off".to_string(),
        };
        let grep = self.state.grep.as_deref().unwrap_or("");
        let follow = if self.state.follow_mode { "on" } else { "off" };

        let spans = vec![
            Span::styled(" filters  ", Style::default().fg(Color::DarkGray)),
            Span::raw("svc="),
            Span::styled(services, Style::default().fg(Color::Cyan)),
            Span::raw("  range="),
            Span::styled(range, Style::default().fg(Color::Yellow)),
            Span::raw("  search="),
            Span::styled(search, Style::default().fg(Color::Magenta)),
            Span::raw("  grep="),
            Span::styled(
                if grep.is_empty() { "—" } else { grep },
                Style::default().fg(Color::Magenta),
            ),
            Span::raw("  follow:"),
            Span::styled(
                follow,
                Style::default().fg(if self.state.follow_mode {
                    Color::Green
                } else {
                    Color::DarkGray
                }),
            ),
        ];

        frame.render_widget(
            Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::BOTTOM)),
            area,
        );
    }

    fn render_list(&mut self, area: Rect, frame: &mut ratatui::Frame) {
        if self.entries.is_empty() {
            let msg = if self.loading_history {
                "Loading..."
            } else {
                "No log entries in the selected window."
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!("  {}", msg),
                    Style::default().fg(Color::DarkGray),
                ))),
                area,
            );
            return;
        }

        let items: Vec<ListItem> = self
            .entries
            .iter()
            .map(|e| ListItem::new(format_entry_line(e, self.search_regex.as_ref())))
            .collect();

        let list = List::new(items)
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            );

        frame.render_stateful_widget(list, area, &mut self.list_state);
    }

    fn render_input_or_help(&self, area: Rect, frame: &mut ratatui::Frame) {
        let line = match self.input_mode {
            InputMode::Search => Line::from(vec![
                Span::styled(" /", Style::default().fg(Color::Magenta)),
                Span::raw(&self.input_buffer),
                Span::styled("_", Style::default().add_modifier(Modifier::SLOW_BLINK)),
            ]),
            InputMode::Normal => Line::from(Span::styled(
                " j/k:scroll  PgUp/Dn:page  g/G:top/end  /:search  n/N:next/prev  f:follow  q:quit",
                Style::default().fg(Color::DarkGray),
            )),
        };
        frame.render_widget(Paragraph::new(line), area);
    }

    fn render_status(&self, area: Rect, frame: &mut ratatui::Frame) {
        let pos = match self.list_state.selected() {
            Some(i) => format!("{}/{}", i + 1, self.entries.len()),
            None => format!("—/{}", self.entries.len()),
        };
        let loading = match (self.loading_history, self.following) {
            (true, true) => "[loading • following]",
            (true, false) => "[loading]",
            (false, true) => "[following]",
            _ => "",
        };
        let err = self.last_error.as_deref().unwrap_or("");

        let mut spans = vec![
            Span::raw(" "),
            Span::styled(pos, Style::default().fg(Color::Cyan)),
        ];
        if !loading.is_empty() {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(loading, Style::default().fg(Color::Yellow)));
        }
        if !err.is_empty() {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!("error: {}", err),
                Style::default().fg(Color::Red),
            ));
        }

        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

fn format_entry_line<'a>(entry: &'a LogEntry, search: Option<&regex::Regex>) -> Line<'a> {
    // Compact time column: "HH:MM:SS.mmm" (12 chars) when we have a timestamp.
    // When we DON'T have one we drop the column entirely instead of padding —
    // a left gutter of blank cells reads as "huge black space" in most themes.
    let mut spans: Vec<Span<'a>> = Vec::with_capacity(6);
    if let Some(t) = entry.ts() {
        spans.push(Span::styled(
            t.format("%H:%M:%S%.3f").to_string(),
            Style::default().fg(Color::DarkGray),
        ));
        spans.push(Span::raw(" "));
    }
    let service_color = ratatui_color(crate::cloud::service_color(&entry.service));
    // Left-align the service inside an 8-char field. Trailing padding keeps
    // the message column aligned across rows without creating a leading
    // dead zone for short service names like "ssp".
    let service_label = if entry.service.len() >= 8 {
        entry.service.clone()
    } else {
        format!("{:<8}", entry.service)
    };
    spans.push(Span::styled(
        service_label,
        Style::default().fg(service_color),
    ));
    spans.push(Span::raw(" "));

    match search {
        Some(re) => {
            let message = entry.message.clone();
            let mut cursor = 0;
            for m in re.find_iter(&message) {
                if m.start() > cursor {
                    spans.push(Span::raw(message[cursor..m.start()].to_string()));
                }
                spans.push(Span::styled(
                    message[m.start()..m.end()].to_string(),
                    Style::default()
                        .bg(Color::Yellow)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                ));
                cursor = m.end();
            }
            if cursor < message.len() {
                spans.push(Span::raw(message[cursor..].to_string()));
            }
        }
        None => {
            spans.push(Span::raw(entry.message.clone()));
        }
    }

    Line::from(spans)
}

fn ratatui_color(c: crossterm::style::Color) -> Color {
    use crossterm::style::Color as CC;
    match c {
        CC::Cyan => Color::Cyan,
        CC::Yellow => Color::Yellow,
        CC::Green => Color::Green,
        CC::Magenta => Color::Magenta,
        CC::Blue => Color::Blue,
        CC::Red => Color::Red,
        CC::DarkGrey | CC::Grey => Color::DarkGray,
        _ => Color::White,
    }
}

fn short_ts(ts: DateTime<Utc>) -> String {
    ts.format("%m-%d %H:%M").to_string()
}

// ---------------------------------------------------------------------------
// Background fetchers
// ---------------------------------------------------------------------------

enum FetchKind {
    Initial,
    Older,
}

fn spawn_history_fetch(
    state: &AppState,
    since: DateTime<Utc>,
    until: Option<DateTime<Utc>>,
    tx: Sender<FetchMsg>,
    kind: FetchKind,
) {
    let url = build_logs_url(
        &state.base_url,
        &state.pid,
        state.services.as_deref(),
        Some(since),
        until,
        state.grep.as_deref(),
        /*tail=*/ false,
        Some(SCROLL_BACK_CHUNK),
    );
    let auth = state.auth_header.clone();
    let idle_deadline = Duration::from_secs(5);

    thread::spawn(move || {
        let mut batch: Vec<LogEntry> = Vec::new();
        let result = stream_entries(
            &url,
            &auth,
            |entry| {
                batch.push(entry);
                StreamAction::Continue
            },
            Some(idle_deadline),
        );

        // Emit whatever we collected.
        match kind {
            FetchKind::Initial => {
                if !batch.is_empty() {
                    let _ = tx.send(FetchMsg::Appended { entries: batch });
                }
            }
            FetchKind::Older => {
                if !batch.is_empty() {
                    let _ = tx.send(FetchMsg::Older { entries: batch });
                }
            }
        }

        if let Err(e) = result {
            let _ = tx.send(FetchMsg::Error(e));
        }
        let _ = tx.send(FetchMsg::HistoryDone);
    });
}

fn spawn_follow_stream(
    state: &AppState,
    since: DateTime<Utc>,
    tx: Sender<FetchMsg>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
) {
    let url = build_logs_url(
        &state.base_url,
        &state.pid,
        state.services.as_deref(),
        Some(since),
        None,
        state.grep.as_deref(),
        /*tail=*/ true,
        None,
    );
    let auth = state.auth_header.clone();

    thread::spawn(move || {
        // Batch entries on short ticks so the UI doesn't redraw per line.
        let buffer: Arc<Mutex<Vec<LogEntry>>> = Arc::new(Mutex::new(Vec::new()));
        let flush_tx = tx.clone();
        let flush_buf = buffer.clone();
        let flush_cancel = cancel.clone();
        let flusher = thread::spawn(move || {
            while !flush_cancel.load(std::sync::atomic::Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(150));
                let drained: Vec<LogEntry> = {
                    let mut b = flush_buf.lock().unwrap();
                    std::mem::take(&mut *b)
                };
                if !drained.is_empty() {
                    if flush_tx.send(FetchMsg::Appended { entries: drained }).is_err() {
                        break;
                    }
                }
            }
        });

        let result = stream_entries(&url, &auth, |entry| {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return StreamAction::Stop;
            }
            buffer.lock().unwrap().push(entry);
            StreamAction::Continue
        }, None);

        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = flusher.join();

        // Final flush.
        let drained: Vec<LogEntry> = {
            let mut b = buffer.lock().unwrap();
            std::mem::take(&mut *b)
        };
        if !drained.is_empty() {
            let _ = tx.send(FetchMsg::Appended { entries: drained });
        }

        if let Err(e) = result {
            let _ = tx.send(FetchMsg::Error(e));
        }
        let _ = tx.send(FetchMsg::FollowClosed);
    });
}

enum StreamAction {
    Continue,
    Stop,
}

/// Open the SSE stream and call `on_entry` for each parsed log line.
/// If `idle_deadline` is set, returns cleanly when no entry arrives within
/// that window — useful for bounded historical fetches where the server may
/// or may not close the stream.
fn stream_entries<F>(
    url: &str,
    auth: &str,
    mut on_entry: F,
    idle_deadline: Option<Duration>,
) -> std::result::Result<(), String>
where
    F: FnMut(LogEntry) -> StreamAction,
{
    let resp = ureq::get(url)
        .set("Authorization", auth)
        .set("Accept", "text/event-stream")
        .timeout(Duration::from_secs(30))
        .call();

    let resp = match resp {
        Ok(r) => r,
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            return Err(format!("HTTP {}: {}", code, body));
        }
        Err(ureq::Error::Transport(t)) => {
            return Err(format!("connection error: {}", t));
        }
    };

    let reader = std::io::BufReader::with_capacity(512, resp.into_reader());
    let mut last_entry = Instant::now();
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => return Err(format!("read error: {}", e)),
        };
        if let Some(data) = line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:")) {
            if let Some(entry) = crate::cloud::parse_log_line(data) {
                last_entry = Instant::now();
                match on_entry(entry) {
                    StreamAction::Continue => {}
                    StreamAction::Stop => return Ok(()),
                }
            }
        }
        if let Some(deadline) = idle_deadline {
            if last_entry.elapsed() > deadline {
                return Ok(());
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// URL builder
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn build_logs_url(
    base_url: &str,
    pid: &str,
    services: Option<&[String]>,
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
    grep: Option<&str>,
    tail: bool,
    limit: Option<usize>,
) -> String {
    let mut params: Vec<(String, String)> = Vec::new();
    if let Some(svc) = services {
        if !svc.is_empty() {
            params.push(("service".to_string(), svc.join(",")));
        }
    }
    if let Some(ts) = since {
        params.push(("since".to_string(), ts.to_rfc3339()));
    }
    if let Some(ts) = until {
        params.push(("until".to_string(), ts.to_rfc3339()));
    }
    if let Some(g) = grep {
        params.push(("q".to_string(), g.to_string()));
    }
    if !tail {
        params.push(("tail".to_string(), "false".to_string()));
    }
    if let Some(l) = limit {
        params.push(("limit".to_string(), l.to_string()));
    }

    let qs: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{}={}", crate::cloud::urlencode(k), crate::cloud::urlencode(v)))
        .collect();

    if qs.is_empty() {
        format!("{}/v1/projects/{}/logs", base_url, pid)
    } else {
        format!("{}/v1/projects/{}/logs?{}", base_url, pid, qs.join("&"))
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Wrapper used by `cloud::logs()` when `--interactive` is set.
/// Split so `run()` can consume a self-contained `LogsBrowserArgs` struct
/// without any dependency on `cloud::*` private types.
pub fn launch(
    client_base_url: String,
    auth_header: String,
    pid: String,
    services: Option<Vec<String>>,
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
    grep: Option<String>,
    follow: bool,
) -> Result<()> {
    run(LogsBrowserArgs {
        base_url: client_base_url,
        auth_header,
        pid,
        services,
        since,
        until,
        grep,
        follow,
    })
    .context("interactive log browser failed")
}

