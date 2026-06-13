//! `spky jobs` — operator overview of the outbox job system, with kill/retry.
//!
//! Reads jobs straight from the configured SurrealDB (like `spky flag`): the
//! per-row data and aggregate metrics come from `SELECT`s against each job table.
//! Mutations (`kill`/`retry`) cannot be done with a plain `UPDATE` — job pickup is
//! gated on CREATE inside the SSP — so they call the `fn::job::kill` /
//! `fn::job::retry` SurrealQL functions, which proxy to the SSP/scheduler over
//! HTTP. Those functions are installed by the schema deploy
//! (`functions_remote_singlenode.surql`).
//!
//! Job tables are discovered from `sp00ky.yml` (`apps.*` of type `backend` with an
//! `outbox` method), falling back to `job`. The CLI deliberately does not depend on
//! the `job-runner` crate (it pins a different surrealdb-core major), so the small
//! discovery parse is replicated here on top of the CLI's own config types.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Stdout;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap};
use ratatui::{Frame, Terminal};

use crate::backend::{self, AppType, DEFAULT_CONFIG_PATH};
use crate::surreal_client::{MigrationDB, SurrealClient};
use crate::{ConnectionArgs, JobsCommands};

const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

/// All four terminal/non-terminal job statuses, in display order.
const STATUS_ORDER: [&str; 4] = ["pending", "processing", "success", "failed"];

pub fn run(
    conn: ConnectionArgs,
    config: Option<PathBuf>,
    action: Option<JobsCommands>,
) -> Result<()> {
    let client = client_from(&conn, &config);
    let tables = discover_job_tables(&config);

    match action {
        None => run_tui(client, tables),
        Some(JobsCommands::List {
            status,
            table,
            limit,
            json,
        }) => list(&client, &tables, status, table, limit, json),
        Some(JobsCommands::Get { id, json }) => get(&client, id, json),
        Some(JobsCommands::Kill { id }) => kill(&client, &id),
        Some(JobsCommands::Retry { id }) => retry(&client, &id),
    }
}

// =============================================================
// Connection + job-table discovery
// =============================================================

fn client_from(conn: &ConnectionArgs, config: &Option<PathBuf>) -> SurrealClient {
    let config_file = config
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH));
    let sp00ky_config = backend::load_config(&config_file);
    let resolved = sp00ky_config.resolved_surrealdb();

    let namespace = if conn.namespace == "main" {
        resolved.namespace
    } else {
        conn.namespace.clone()
    };
    let database = if conn.database == "main" {
        resolved.database
    } else {
        conn.database.clone()
    };

    SurrealClient::new(
        &conn.url,
        &namespace,
        &database,
        &conn.username,
        &conn.password,
    )
}

/// Map of job table name -> backend app name, parsed from `sp00ky.yml`. Any
/// `backend` app with an `outbox` method that names a `table` is a job table.
/// Falls back to a single `job` table if none are configured.
fn discover_job_tables(config: &Option<PathBuf>) -> BTreeMap<String, String> {
    let config_file = config
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH));
    let cfg = backend::load_config(&config_file);

    let mut out = BTreeMap::new();
    for (name, app) in &cfg.apps {
        if app.app_type != AppType::Backend {
            continue;
        }
        if let Some(method) = &app.method {
            if let Some(table) = &method.table {
                out.insert(table.clone(), name.clone());
            }
        }
    }
    if out.is_empty() {
        out.insert("job".to_string(), "job".to_string());
    }
    out
}

// =============================================================
// SurrealQL helpers
// =============================================================

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Pull the array of rows out of a single SurrealDB statement response.
fn result_rows(result: Option<&Value>) -> Vec<Value> {
    match result {
        Some(Value::Array(arr)) => arr.clone(),
        Some(other) => vec![other.clone()],
        None => vec![],
    }
}

// =============================================================
// Data model
// =============================================================

#[derive(Clone)]
struct JobRow {
    id: String,
    table: String,
    backend: String,
    status: String,
    path: String,
    retries: i64,
    max_retries: i64,
    retry_strategy: String,
    created_at: String,
    updated_at: String,
    errors: Vec<Value>,
    payload: Value,
}

impl JobRow {
    fn last_error(&self) -> Option<String> {
        self.errors.last().map(|e| {
            let reason = e.get("reason").and_then(Value::as_str).unwrap_or("");
            let code = e
                .get("code")
                .map(|c| match c {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_default();
            if code.is_empty() {
                reason.to_string()
            } else {
                format!("[{}] {}", code, reason)
            }
        })
    }

    fn age(&self) -> Option<String> {
        parse_ts(&self.created_at).map(fmt_age)
    }
}

#[derive(Default, Clone)]
struct StatusCounts {
    pending: i64,
    processing: i64,
    success: i64,
    failed: i64,
    other: i64,
}

impl StatusCounts {
    fn add(&mut self, status: &str, n: i64) {
        match status {
            "pending" => self.pending += n,
            "processing" => self.processing += n,
            "success" => self.success += n,
            "failed" => self.failed += n,
            _ => self.other += n,
        }
    }

    fn total(&self) -> i64 {
        self.pending + self.processing + self.success + self.failed + self.other
    }

    /// Failed as a fraction of all terminal jobs (success + failed).
    fn fail_rate(&self) -> f64 {
        let terminal = self.success + self.failed;
        if terminal == 0 {
            0.0
        } else {
            self.failed as f64 / terminal as f64 * 100.0
        }
    }
}

struct Snapshot {
    rows: Vec<JobRow>,
    counts: StatusCounts,
    throughput_1m: i64,
    oldest_pending: Option<DateTime<Utc>>,
    errors: Vec<String>,
}

// =============================================================
// Fetching
// =============================================================

/// Query one job table in a single round-trip (rows + status counts + 1-minute
/// success throughput + oldest pending timestamp).
fn fetch_table(
    client: &SurrealClient,
    table: &str,
    backend_name: &str,
    limit: usize,
) -> Result<(Vec<JobRow>, StatusCounts, i64, Option<DateTime<Utc>>)> {
    let query = format!(
        "SELECT type::string(id) AS id, status, path, retries, max_retries, retry_strategy, \
         type::string(created_at) AS created_at, type::string(updated_at) AS updated_at, errors, payload \
         FROM {table} ORDER BY updated_at DESC LIMIT {limit}; \
         SELECT status, count() AS n FROM {table} GROUP BY status; \
         SELECT count() AS n FROM {table} WHERE status = 'success' AND updated_at > time::now() - 1m GROUP ALL; \
         SELECT VALUE type::string(created_at) FROM {table} WHERE status = 'pending' ORDER BY created_at ASC LIMIT 1;",
        table = table,
        limit = limit
    );

    let responses = client
        .execute(&query)
        .with_context(|| format!("Failed to query job table '{}'", table))?;
    let results: Vec<Option<Value>> = responses.into_iter().map(|r| r.result).collect();

    let rows = result_rows(results.first().and_then(|r| r.as_ref()))
        .iter()
        .filter_map(|v| parse_job_row(v, table, backend_name))
        .collect::<Vec<_>>();

    let mut counts = StatusCounts::default();
    for v in result_rows(results.get(1).and_then(|r| r.as_ref())) {
        let status = v.get("status").and_then(Value::as_str).unwrap_or("");
        let n = v.get("n").and_then(Value::as_i64).unwrap_or(0);
        counts.add(status, n);
    }

    let throughput = result_rows(results.get(2).and_then(|r| r.as_ref()))
        .first()
        .and_then(|v| v.get("n"))
        .and_then(Value::as_i64)
        .unwrap_or(0);

    let oldest_pending = result_rows(results.get(3).and_then(|r| r.as_ref()))
        .first()
        .and_then(Value::as_str)
        .and_then(parse_ts);

    Ok((rows, counts, throughput, oldest_pending))
}

/// Fetch and merge a snapshot across every configured job table. A failure on one
/// table is recorded but does not abort the others (e.g. a table that does not
/// exist yet on a fresh database).
fn fetch_snapshot(
    client: &SurrealClient,
    tables: &BTreeMap<String, String>,
    limit: usize,
) -> Snapshot {
    let mut snapshot = Snapshot {
        rows: Vec::new(),
        counts: StatusCounts::default(),
        throughput_1m: 0,
        oldest_pending: None,
        errors: Vec::new(),
    };

    for (table, backend_name) in tables {
        match fetch_table(client, table, backend_name, limit) {
            Ok((rows, counts, throughput, oldest)) => {
                snapshot.rows.extend(rows);
                snapshot.counts.pending += counts.pending;
                snapshot.counts.processing += counts.processing;
                snapshot.counts.success += counts.success;
                snapshot.counts.failed += counts.failed;
                snapshot.counts.other += counts.other;
                snapshot.throughput_1m += throughput;
                if let Some(o) = oldest {
                    snapshot.oldest_pending =
                        Some(snapshot.oldest_pending.map(|cur| cur.min(o)).unwrap_or(o));
                }
            }
            Err(e) => snapshot.errors.push(format!("{}: {}", table, e)),
        }
    }

    // Merge-sort by updated_at descending. ISO-8601 strings sort chronologically.
    snapshot
        .rows
        .sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    snapshot
}

fn parse_job_row(v: &Value, table: &str, backend_name: &str) -> Option<JobRow> {
    let obj = v.as_object()?;
    let id = obj.get("id").and_then(Value::as_str)?.to_string();
    Some(JobRow {
        id,
        table: table.to_string(),
        backend: backend_name.to_string(),
        status: obj
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        path: obj
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        retries: obj.get("retries").and_then(Value::as_i64).unwrap_or(0),
        max_retries: obj.get("max_retries").and_then(Value::as_i64).unwrap_or(0),
        retry_strategy: obj
            .get("retry_strategy")
            .and_then(Value::as_str)
            .unwrap_or("linear")
            .to_string(),
        created_at: obj
            .get("created_at")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        updated_at: obj
            .get("updated_at")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        errors: obj
            .get("errors")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        payload: obj.get("payload").cloned().unwrap_or(Value::Null),
    })
}

// =============================================================
// Formatting helpers
// =============================================================

fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Compact age from a past timestamp ("3s", "4m", "2h", "5d").
fn fmt_age(ts: DateTime<Utc>) -> String {
    fmt_secs((Utc::now() - ts).num_seconds().max(0))
}

fn fmt_secs(secs: i64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let kept: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", kept)
    }
}

// =============================================================
// `spky jobs list`
// =============================================================

fn list(
    client: &SurrealClient,
    tables: &BTreeMap<String, String>,
    status: Option<String>,
    table: Option<String>,
    limit: usize,
    json: bool,
) -> Result<()> {
    let snapshot = fetch_snapshot(client, tables, limit);

    let rows: Vec<&JobRow> = snapshot
        .rows
        .iter()
        .filter(|r| status.as_deref().map(|s| r.status == s).unwrap_or(true))
        .filter(|r| table.as_deref().map(|t| r.table == t).unwrap_or(true))
        .collect();

    if json {
        let arr: Vec<Value> = rows
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id,
                    "table": r.table,
                    "backend": r.backend,
                    "status": r.status,
                    "path": r.path,
                    "retries": r.retries,
                    "max_retries": r.max_retries,
                    "retry_strategy": r.retry_strategy,
                    "created_at": r.created_at,
                    "updated_at": r.updated_at,
                    "errors": r.errors,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&Value::Array(arr))?);
        return Ok(());
    }

    // Aggregate metrics line.
    let c = &snapshot.counts;
    println!(
        "{BOLD}jobs{RESET}  {YELLOW}{} pending{RESET}  {CYAN}{} processing{RESET}  {GREEN}{} success{RESET}  {RED}{} failed{RESET}  {DIM}({} total){RESET}",
        c.pending, c.processing, c.success, c.failed, c.total()
    );
    let oldest = snapshot
        .oldest_pending
        .map(|t| fmt_age(t))
        .unwrap_or_else(|| "-".to_string());
    println!(
        "{DIM}fail rate {:.1}%   throughput {} ✓/min   oldest pending {}{RESET}",
        c.fail_rate(),
        snapshot.throughput_1m,
        oldest
    );
    for e in &snapshot.errors {
        println!("{YELLOW}warn:{RESET} {}", e);
    }
    println!();

    if rows.is_empty() {
        println!("{DIM}No jobs found.{RESET}");
        return Ok(());
    }

    println!(
        "{BOLD}{:<24} {:<10} {:<16} {:<12} {:>5}  {:>5}  {}{RESET}",
        "ID", "BACKEND", "PATH", "STATUS", "RETRY", "AGE", "LAST ERROR"
    );
    for r in rows {
        let (icon, color) = status_style(&r.status);
        println!(
            "{:<24} {:<10} {:<16} {}{} {:<10}{RESET} {:>5}  {:>5}  {DIM}{}{RESET}",
            truncate(&r.id, 24),
            truncate(&r.backend, 10),
            truncate(&r.path, 16),
            color,
            icon,
            r.status,
            format!("{}/{}", r.retries, r.max_retries),
            r.age().unwrap_or_else(|| "-".to_string()),
            truncate(&r.last_error().unwrap_or_default(), 50),
        );
    }
    Ok(())
}

/// Plain (non-ANSI) status icon for terminals and the TUI.
fn status_icon(status: &str) -> &'static str {
    match status {
        "pending" => "⏳",
        "processing" => "●",
        "success" => "✓",
        "failed" => "✗",
        _ => "·",
    }
}

fn status_style(status: &str) -> (&'static str, &'static str) {
    match status {
        "pending" => ("⏳", YELLOW),
        "processing" => ("●", CYAN),
        "success" => ("✓", GREEN),
        "failed" => ("✗", RED),
        _ => ("·", DIM),
    }
}

// =============================================================
// `spky jobs get`
// =============================================================

fn get(client: &SurrealClient, id: String, json: bool) -> Result<()> {
    let (tb, key) = id
        .split_once(':')
        .ok_or_else(|| anyhow!("Job id must be 'table:key', got '{}'", id))?;
    let query = format!(
        "SELECT type::string(id) AS id, status, path, retries, max_retries, retry_strategy, \
         type::string(created_at) AS created_at, type::string(updated_at) AS updated_at, errors, payload \
         FROM type::record('{}', '{}');",
        esc(tb),
        esc(key)
    );
    let responses = client.execute(&query).context("Failed to load job")?;
    let row = responses
        .into_iter()
        .next()
        .and_then(|r| r.result)
        .and_then(|r| result_rows(Some(&r)).into_iter().next())
        .ok_or_else(|| anyhow!("Job '{}' not found", id))?;

    if json {
        println!("{}", serde_json::to_string_pretty(&row)?);
        return Ok(());
    }

    let job = parse_job_row(&row, tb, tb).ok_or_else(|| anyhow!("Unexpected job shape"))?;
    let (icon, color) = status_style(&job.status);
    println!("{BOLD}{}{RESET}", job.id);
    println!("  status      : {}{} {}{RESET}", color, icon, job.status);
    println!("  backend     : {}", job.backend);
    println!("  path        : {}", job.path);
    println!("  retries     : {}/{} ({})", job.retries, job.max_retries, job.retry_strategy);
    println!("  created     : {} {DIM}({} ago){RESET}", job.created_at, job.age().unwrap_or_else(|| "-".into()));
    println!("  updated     : {}", job.updated_at);
    println!("  payload     : {}", serde_json::to_string(&job.payload).unwrap_or_default());
    if job.errors.is_empty() {
        println!("  errors      : {DIM}none{RESET}");
    } else {
        println!("  errors      : {} attempt(s)", job.errors.len());
        for (i, e) in job.errors.iter().enumerate() {
            let code = e.get("code").map(|c| c.to_string()).unwrap_or_default();
            let reason = e.get("reason").and_then(Value::as_str).unwrap_or("");
            println!("    {DIM}#{} [{}]{RESET} {}", i + 1, code, reason);
        }
    }
    Ok(())
}

// =============================================================
// `spky jobs kill` / `spky jobs retry`
// =============================================================

fn kill(client: &SurrealClient, id: &str) -> Result<()> {
    let result = call_job_fn(client, "kill", id)?;
    print_action_result("Killed", id, &result);
    Ok(())
}

fn retry(client: &SurrealClient, id: &str) -> Result<()> {
    let result = call_job_fn(client, "retry", id)?;
    print_action_result("Retrying", id, &result);
    Ok(())
}

/// Invoke `fn::job::<action>($id)` and return the (parsed) backend response value.
fn call_job_fn(client: &SurrealClient, action: &str, id: &str) -> Result<Value> {
    let query = format!("RETURN fn::job::{}('{}');", action, esc(id));
    let responses = client
        .execute(&query)
        .with_context(|| format!("Failed to {} job '{}'", action, id))?;
    Ok(responses
        .into_iter()
        .next()
        .and_then(|r| r.result)
        .unwrap_or(Value::Null))
}

fn action_message(result: &Value) -> Option<String> {
    result
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn print_action_result(verb: &str, id: &str, result: &Value) {
    match action_message(result) {
        Some(msg) => println!("{GREEN}{} {}{RESET} — {}", verb, id, msg),
        None => println!("{GREEN}{} {}{RESET}", verb, id),
    }
}

// =============================================================
// Interactive TUI (`spky jobs`)
// =============================================================

const TICK: Duration = Duration::from_millis(200);
const REFRESH_INTERVAL: Duration = Duration::from_millis(1500);
const TUI_LIMIT: usize = 200;

struct TuiApp {
    snapshot: Snapshot,
    /// `None` = show all; otherwise one of STATUS_ORDER.
    filter: Option<&'static str>,
    table_state: TableState,
    detail_open: bool,
    last_refresh: Instant,
    status_msg: Option<String>,
}

impl TuiApp {
    fn new() -> Self {
        let mut table_state = TableState::default();
        table_state.select(Some(0));
        Self {
            snapshot: Snapshot {
                rows: Vec::new(),
                counts: StatusCounts::default(),
                throughput_1m: 0,
                oldest_pending: None,
                errors: Vec::new(),
            },
            filter: None,
            table_state,
            detail_open: false,
            last_refresh: Instant::now(),
            status_msg: None,
        }
    }

    fn refresh(&mut self, client: &SurrealClient, tables: &BTreeMap<String, String>) {
        self.snapshot = fetch_snapshot(client, tables, TUI_LIMIT);
        self.last_refresh = Instant::now();
        self.clamp_selection();
    }

    fn visible(&self) -> Vec<&JobRow> {
        self.snapshot
            .rows
            .iter()
            .filter(|r| self.filter.map(|f| r.status == f).unwrap_or(true))
            .collect()
    }

    fn clamp_selection(&mut self) {
        let len = self.visible().len();
        if len == 0 {
            self.table_state.select(None);
        } else {
            let sel = self.table_state.selected().unwrap_or(0).min(len - 1);
            self.table_state.select(Some(sel));
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.visible().len();
        if len == 0 {
            return;
        }
        let cur = self.table_state.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, len as isize - 1);
        self.table_state.select(Some(next as usize));
    }

    fn selected_id(&self) -> Option<String> {
        let visible = self.visible();
        self.table_state
            .selected()
            .and_then(|i| visible.get(i))
            .map(|r| r.id.clone())
    }

    fn cycle_filter(&mut self) {
        self.filter = match self.filter {
            None => Some(STATUS_ORDER[0]),
            Some(cur) => {
                let idx = STATUS_ORDER.iter().position(|s| *s == cur).unwrap_or(0);
                if idx + 1 < STATUS_ORDER.len() {
                    Some(STATUS_ORDER[idx + 1])
                } else {
                    None
                }
            }
        };
        self.clamp_selection();
    }
}

fn run_tui(client: SurrealClient, tables: BTreeMap<String, String>) -> Result<()> {
    // Restore the terminal even if a panic unwinds through the draw loop.
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

    let result = tui_loop(&mut terminal, &client, &tables);

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::cursor::Show
    )?;

    result
}

fn tui_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    client: &SurrealClient,
    tables: &BTreeMap<String, String>,
) -> Result<()> {
    let mut app = TuiApp::new();
    app.refresh(client, tables);

    loop {
        terminal.draw(|f| render(f, &mut app))?;

        if event::poll(TICK)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        if app.detail_open {
                            app.detail_open = false;
                        } else {
                            break;
                        }
                    }
                    KeyCode::Up => app.move_selection(-1),
                    KeyCode::Down => app.move_selection(1),
                    KeyCode::Enter => app.detail_open = !app.detail_open,
                    KeyCode::Char('f') => app.cycle_filter(),
                    KeyCode::Char('k') => {
                        if let Some(id) = app.selected_id() {
                            app.status_msg = Some(match call_job_fn(client, "kill", &id) {
                                Ok(v) => format!(
                                    "kill {} — {}",
                                    id,
                                    action_message(&v).unwrap_or_else(|| "ok".into())
                                ),
                                Err(e) => format!("kill {} failed: {}", id, e),
                            });
                            app.refresh(client, tables);
                        }
                    }
                    KeyCode::Char('r') => {
                        if let Some(id) = app.selected_id() {
                            app.status_msg = Some(match call_job_fn(client, "retry", &id) {
                                Ok(v) => format!(
                                    "retry {} — {}",
                                    id,
                                    action_message(&v).unwrap_or_else(|| "ok".into())
                                ),
                                Err(e) => format!("retry {} failed: {}", id, e),
                            });
                            app.refresh(client, tables);
                        }
                    }
                    _ => {}
                }
            }
        }

        if app.last_refresh.elapsed() >= REFRESH_INTERVAL {
            app.refresh(client, tables);
        }
    }

    Ok(())
}

fn render(f: &mut Frame, app: &mut TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // metrics
            Constraint::Min(0),    // table
            Constraint::Length(1), // footer
        ])
        .split(f.area());

    render_metrics(f, chunks[0], app);
    render_table(f, chunks[1], app);
    render_footer(f, chunks[2], app);

    if app.detail_open {
        render_detail(f, app);
    }
}

fn render_metrics(f: &mut Frame, area: Rect, app: &TuiApp) {
    let c = &app.snapshot.counts;
    let oldest = app
        .snapshot
        .oldest_pending
        .map(fmt_age)
        .unwrap_or_else(|| "-".to_string());

    let counts_line = Line::from(vec![
        Span::styled(
            format!("{} pending", c.pending),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw("   "),
        Span::styled(
            format!("{} processing", c.processing),
            Style::default().fg(Color::Cyan),
        ),
        Span::raw("   "),
        Span::styled(
            format!("{} success", c.success),
            Style::default().fg(Color::Green),
        ),
        Span::raw("   "),
        Span::styled(
            format!("{} failed", c.failed),
            Style::default().fg(Color::Red),
        ),
        Span::styled(
            format!("   ({} total)", c.total()),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    let detail_line = Line::from(Span::styled(
        format!(
            "fail rate {:.1}%    throughput {} ✓/min    oldest pending {}",
            c.fail_rate(),
            app.snapshot.throughput_1m,
            oldest
        ),
        Style::default().fg(Color::DarkGray),
    ));

    let filter_label = app.filter.unwrap_or("all");
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            format!(" spky jobs · filter: {} ", filter_label),
            Style::default().add_modifier(Modifier::BOLD),
        ));
    let para = Paragraph::new(vec![counts_line, detail_line]).block(block);
    f.render_widget(para, area);
}

fn render_table(f: &mut Frame, area: Rect, app: &mut TuiApp) {
    let visible = app.visible();
    if visible.is_empty() {
        let msg = if app.snapshot.rows.is_empty() {
            "No jobs found."
        } else {
            "No jobs match the current filter (press 'f' to cycle)."
        };
        let para = Paragraph::new(msg)
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        f.render_widget(para, area);
        return;
    }

    let header = Row::new(vec![
        "ID", "BACKEND", "PATH", "STATUS", "RETRY", "AGE", "LAST ERROR",
    ])
    .style(Style::default().add_modifier(Modifier::BOLD))
    .bottom_margin(0);

    let rows: Vec<Row> = visible
        .iter()
        .map(|r| {
            let color = match r.status.as_str() {
                "pending" => Color::Yellow,
                "processing" => Color::Cyan,
                "success" => Color::Green,
                "failed" => Color::Red,
                _ => Color::DarkGray,
            };
            Row::new(vec![
                Cell::from(truncate(&r.id, 22)),
                Cell::from(truncate(&r.backend, 10)),
                Cell::from(truncate(&r.path, 16)),
                Cell::from(Span::styled(
                    format!("{} {}", status_icon(&r.status), r.status),
                    Style::default().fg(color),
                )),
                Cell::from(format!("{}/{}", r.retries, r.max_retries)),
                Cell::from(r.age().unwrap_or_else(|| "-".to_string())),
                Cell::from(Span::styled(
                    truncate(&r.last_error().unwrap_or_default(), 40),
                    Style::default().fg(Color::DarkGray),
                )),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(22),
        Constraint::Length(10),
        Constraint::Length(16),
        Constraint::Length(13),
        Constraint::Length(6),
        Constraint::Length(5),
        Constraint::Min(10),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(
            Style::default()
                .bg(Color::Indexed(238))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    f.render_stateful_widget(table, area, &mut app.table_state);
}

fn render_footer(f: &mut Frame, area: Rect, app: &TuiApp) {
    let text = match &app.status_msg {
        Some(msg) => Line::from(Span::styled(
            format!(" {} ", msg),
            Style::default().fg(Color::White).bg(Color::Indexed(238)),
        )),
        None => Line::from(Span::styled(
            " ↑/↓ select   k kill   r retry   ⏎ details   f filter   q quit ",
            Style::default().fg(Color::DarkGray),
        )),
    };
    f.render_widget(Paragraph::new(text), area);
}

fn render_detail(f: &mut Frame, app: &TuiApp) {
    let visible = app.visible();
    let Some(job) = app.table_state.selected().and_then(|i| visible.get(i)) else {
        return;
    };

    let area = centered_rect(70, 70, f.area());
    f.render_widget(Clear, area);

    let mut lines = vec![
        kv("status", &job.status),
        kv("backend", &job.backend),
        kv("table", &job.table),
        kv("path", &job.path),
        kv("retries", &format!("{}/{} ({})", job.retries, job.max_retries, job.retry_strategy)),
        kv(
            "created",
            &format!(
                "{}  ({} ago)",
                job.created_at,
                job.age().unwrap_or_else(|| "-".into())
            ),
        ),
        kv("updated", &job.updated_at),
        kv(
            "payload",
            &truncate(&serde_json::to_string(&job.payload).unwrap_or_default(), 200),
        ),
        Line::from(""),
        Line::from(Span::styled(
            format!("errors ({}):", job.errors.len()),
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ];
    if job.errors.is_empty() {
        lines.push(Line::from(Span::styled(
            "  none",
            Style::default().fg(Color::DarkGray),
        )));
    }
    for (i, e) in job.errors.iter().enumerate() {
        let code = e.get("code").map(|c| c.to_string()).unwrap_or_default();
        let reason = e.get("reason").and_then(Value::as_str).unwrap_or("");
        lines.push(Line::from(format!("  #{} [{}] {}", i + 1, code, reason)));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            format!(" {} ", job.id),
            Style::default().add_modifier(Modifier::BOLD),
        ));
    let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    f.render_widget(para, area);
}

fn kv<'a>(key: &'a str, value: &str) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{:<10}", key), Style::default().fg(Color::DarkGray)),
        Span::raw(value.to_string()),
    ])
}

/// Rect centered within `area`, sized as a percentage of it.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}
