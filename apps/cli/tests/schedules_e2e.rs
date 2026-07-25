//! End-to-end test for `spky schedules` / `spky workflows` against a real
//! SurrealDB.
//!
//! This is the only place the CLI's own SurrealQL runs against a real v3 server
//! rather than being asserted as a string, so it is what catches the dialect
//! traps: `⟨⟩`-quoted record keys for hyphenated schedule names, `ORDER BY` on a
//! projected field only, and `type::string(...)` around every datetime the CLI
//! reads back. The engine's SQL is covered separately, against an embedded
//! SurrealDB, in `packages/schedule-core`'s `db_tests`.
//!
//! Marked `#[ignore]` so the default `cargo test` stays fast and CI-safe:
//!
//! ```sh
//! cargo test -p sp00ky-cli --test schedules_e2e -- --ignored
//! ```
//!
//! If Docker is not available the test skips rather than failing, mirroring
//! `flag_sql_e2e`.

use std::process::{Command, Output, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

const IMAGE: &str = "surrealdb/surrealdb:v3.1.2";
const NAME: &str = "spky-schedules-e2e";
const PORT: &str = "18078";
const AUTH: &str = "Basic cm9vdDpyb290"; // base64("root:root")
const NS: &str = "e2e";
const DB: &str = "e2e";

/// The shipped scheduling DDL — the same file a deploy applies.
const SCHEDULE_TABLES: &str = include_str!("../src/schedule_tables.surql");

fn docker_available() -> bool {
    Command::new("docker")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn remove_container() {
    let _ = Command::new("docker")
        .args(["rm", "-f", NAME])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

struct Container;
impl Drop for Container {
    fn drop(&mut self) {
        remove_container();
    }
}

fn try_sql(query: &str) -> Option<String> {
    let url = format!("http://127.0.0.1:{PORT}/sql");
    match ureq::post(&url)
        .set("Authorization", AUTH)
        .set("surreal-ns", NS)
        .set("surreal-db", DB)
        .set("Accept", "application/json")
        .send_string(query)
    {
        Ok(r) => Some(r.into_string().unwrap_or_default()),
        Err(ureq::Error::Status(_, r)) => Some(r.into_string().unwrap_or_default()),
        Err(_) => None,
    }
}

fn sql(query: &str) -> String {
    let body = try_sql(query).expect("SurrealDB query failed (transport error)");
    assert!(
        !body.contains("\"status\":\"ERR\""),
        "statement failed:\n  {query}\n  → {body}"
    );
    body
}

fn spky(args: &[&str]) -> Output {
    let url = format!("http://127.0.0.1:{PORT}");
    Command::new(env!("CARGO_BIN_EXE_spky"))
        .args(args)
        .args(["--url", &url, "--namespace", NS, "--database", DB])
        .stdin(Stdio::null())
        .output()
        .expect("failed to run spky")
}

fn combined(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn ok(args: &[&str]) -> String {
    let out = spky(args);
    let text = combined(&out);
    assert!(out.status.success(), "spky {args:?} failed:\n{text}");
    text
}

fn start_surrealdb() -> Container {
    remove_container();
    let started = Command::new("docker")
        .args([
            "run",
            "-d",
            "--name",
            NAME,
            "-p",
            &format!("{PORT}:8000"),
            IMAGE,
            "start",
            "--user",
            "root",
            "--pass",
            "root",
            "--allow-all",
            "memory",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("failed to invoke docker run");
    assert!(started.success(), "docker run failed for {IMAGE}");
    let guard = Container;

    let deadline = Instant::now() + Duration::from_secs(90);
    while try_sql("RETURN 1;").is_none() {
        assert!(Instant::now() < deadline, "SurrealDB did not become ready within 90s");
        sleep(Duration::from_millis(500));
    }
    guard
}

/// Seed a schedule the way a deploy would, with a deliberately hyphenated name —
/// `_00_schedule:game-sync` parses as a subtraction unless the key is quoted, so
/// every statement the CLI emits has to get that right.
fn seed_schedule() {
    sql(SCHEDULE_TABLES);
    sql(
        "CREATE _00_schedule:⟨game-sync⟩ CONTENT { \
           name: 'game-sync', kind: 'job', every_ms: 300000, \
           target_table: 'job', path: '/syncGames', \
           for_each: 'SELECT id FROM connection', for_each_key: 'id', \
           concurrency: 'skip', spec_hash: 'seed', \
           next_fire_at: time::now() + 5m }; \
         CREATE _00_schedule:nightly CONTENT { \
           name: 'nightly', kind: 'job', cron: '0 3 * * *', \
           target_table: 'job', path: '/cleanup', spec_hash: 'seed' };",
    );
}

#[test]
#[ignore = "requires Docker; run with: cargo test -p sp00ky-cli --test schedules_e2e -- --ignored"]
fn schedules_list_pause_resume_trigger_against_a_real_server() {
    if !docker_available() {
        eprintln!("skipping schedules_e2e: Docker is not available");
        return;
    }
    let _guard = start_surrealdb();
    seed_schedule();

    // list: reads the schedule table plus each schedule's latest run.
    let text = ok(&["schedules", "list"]);
    assert!(text.contains("game-sync"), "list output:\n{text}");
    assert!(text.contains("nightly"), "list output:\n{text}");
    assert!(text.contains("every 5m"), "interval cadence is rendered:\n{text}");
    assert!(text.contains("0 3 * * *"), "cron cadence is rendered:\n{text}");

    // A schedule with no clock yet reads as `planning`, not as an error.
    assert!(text.contains("planning"), "unplanned schedule state:\n{text}");

    // --json stays machine-readable.
    let json = ok(&["schedules", "list", "--json"]);
    let parsed: serde_json::Value = serde_json::from_str(json.trim()).expect("valid JSON");
    assert_eq!(parsed.as_array().map(Vec::len), Some(2));

    // get: definition + clock + recent runs, for the hyphenated name.
    let text = ok(&["schedules", "get", "game-sync"]);
    assert!(text.contains("/syncGames"), "get output:\n{text}");
    assert!(text.contains("SELECT id FROM connection"), "forEach shown:\n{text}");

    // pause writes only `paused`; the operator's intent must be readable back.
    ok(&["schedules", "pause", "game-sync"]);
    let body = sql("SELECT VALUE paused FROM ONLY _00_schedule:⟨game-sync⟩;");
    assert!(body.contains("true"), "pause did not take: {body}");
    let text = ok(&["schedules", "list"]);
    assert!(text.contains("paused"), "list reflects the pause:\n{text}");

    // Pausing wins over triggering, and the CLI says so rather than silently
    // queueing a fire that will never happen.
    let out = spky(&["schedules", "trigger", "game-sync"]);
    assert!(!out.status.success(), "trigger must refuse a paused schedule");
    assert!(combined(&out).contains("paused"), "{}", combined(&out));

    ok(&["schedules", "resume", "game-sync"]);
    let body = sql("SELECT VALUE paused FROM ONLY _00_schedule:⟨game-sync⟩;");
    assert!(body.contains("false"), "resume did not take: {body}");

    // trigger stamps its own field, leaving the cron clock alone.
    let before = sql("SELECT VALUE type::string(next_fire_at) FROM ONLY _00_schedule:⟨game-sync⟩;");
    ok(&["schedules", "trigger", "game-sync"]);
    let body = sql("SELECT VALUE trigger_requested_at != NONE FROM ONLY _00_schedule:⟨game-sync⟩;");
    assert!(body.contains("true"), "trigger was not recorded: {body}");
    let after = sql("SELECT VALUE type::string(next_fire_at) FROM ONLY _00_schedule:⟨game-sync⟩;");
    assert_eq!(before, after, "a manual trigger must not shift the cadence");

    // A typo must fail loudly instead of silently doing nothing.
    let out = spky(&["schedules", "pause", "no-such-schedule"]);
    assert!(!out.status.success(), "pausing an unknown schedule must fail");
    assert!(combined(&out).contains("No schedule named"), "{}", combined(&out));
}

#[test]
#[ignore = "requires Docker; run with: cargo test -p sp00ky-cli --test schedules_e2e -- --ignored"]
fn runs_history_and_workflow_views_against_a_real_server() {
    if !docker_available() {
        eprintln!("skipping schedules_e2e: Docker is not available");
        return;
    }
    let _guard = start_surrealdb();
    seed_schedule();

    // Run history across both kinds, including a suppressed tick.
    sql(
        "CREATE _00_schedule_run:r1 CONTENT { schedule_name: 'game-sync', key: 'connection:alice', \
           fire_at: time::now() - 2m, kind: 'job', status: 'success', trigger: 'cron', \
           finished_at: time::now() - 1m }; \
         CREATE _00_schedule_run:r2 CONTENT { schedule_name: 'game-sync', key: 'connection:bob', \
           fire_at: time::now() - 1m, kind: 'job', status: 'skipped', trigger: 'cron', \
           finished_at: time::now() - 1m }; \
         CREATE _00_schedule_run:r3 CONTENT { schedule_name: 'nightly', key: '', \
           fire_at: time::now(), kind: 'job', status: 'running', trigger: 'manual' };",
    );

    let text = ok(&["schedules", "runs"]);
    assert!(text.contains("game-sync"), "runs output:\n{text}");
    assert!(text.contains("skipped"), "a suppressed tick is visible:\n{text}");
    assert!(text.contains("manual"), "how the run was triggered is visible:\n{text}");

    let text = ok(&["schedules", "runs", "nightly"]);
    assert!(text.contains("nightly"), "filtered runs:\n{text}");
    assert!(!text.contains("game-sync"), "the filter actually filters:\n{text}");

    let text = ok(&["schedules", "runs", "--status", "skipped"]);
    assert!(text.contains("connection:bob"), "status filter:\n{text}");
    assert!(!text.contains("connection:alice"), "status filter excludes:\n{text}");

    // A workflow run with a diamond DAG mid-flight.
    sql(
        "CREATE _00_schedule:report CONTENT { name: 'report', kind: 'workflow', \
           cron: '0 6 1 * *', target_table: 'job', spec_hash: 'seed' }; \
         CREATE _00_workflow_run:wf1 CONTENT { workflow_name: 'report', schedule_name: 'report', \
           target_table: 'job', status: 'running', \
           dag: { on_failure: 'halt', steps: [ \
             { name: 'extract-orders', path: '/a', depends_on: [] }, \
             { name: 'extract-users', path: '/b', depends_on: [] }, \
             { name: 'transform', path: '/c', depends_on: ['extract-orders', 'extract-users'] }, \
             { name: 'notify', path: '/d', depends_on: ['transform'] } ] } }; \
         CREATE _00_step_run:s1 CONTENT { workflow_run: _00_workflow_run:wf1, step: 'extract-orders', \
           depends_on: [], status: 'success', job_id: 'job:wf_1_a', output: { fileId: 'f-1' }, \
           finished_at: time::now() }; \
         CREATE _00_step_run:s2 CONTENT { workflow_run: _00_workflow_run:wf1, step: 'extract-users', \
           depends_on: [], status: 'success', job_id: 'job:wf_1_b', finished_at: time::now() }; \
         CREATE _00_step_run:s3 CONTENT { workflow_run: _00_workflow_run:wf1, step: 'transform', \
           depends_on: ['extract-orders', 'extract-users'], status: 'dispatched', job_id: 'job:wf_1_c' }; \
         CREATE _00_step_run:s4 CONTENT { workflow_run: _00_workflow_run:wf1, step: 'notify', \
           depends_on: ['transform'], status: 'blocked' };",
    );

    let text = ok(&["workflows", "list"]);
    assert!(text.contains("report"), "workflow list:\n{text}");

    let text = ok(&["workflows", "runs"]);
    assert!(text.contains("report"), "workflow runs:\n{text}");
    assert!(text.contains("running"), "workflow runs:\n{text}");

    // `show` by workflow name resolves to the newest run and draws the DAG.
    // Output is piped here, so it comes back as plain ASCII.
    let text = ok(&["workflows", "show", "report"]);
    for step in ["extract-orders", "extract-users", "transform", "notify"] {
        assert!(text.contains(step), "{step} missing from the diagram:\n{text}");
    }
    assert!(text.contains('+') && text.contains('-'), "ASCII boxes when piped:\n{text}");

    // `show --json` exposes the same state, including a captured step output.
    let json = ok(&["workflows", "show", "report", "--json"]);
    let parsed: serde_json::Value = serde_json::from_str(json.trim()).expect("valid JSON");
    assert_eq!(parsed["status"], "running");
    assert_eq!(parsed["steps"].as_array().map(Vec::len), Some(4));
    let orders = parsed["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["step"] == "extract-orders")
        .expect("step present");
    assert_eq!(orders["output"]["fileId"], "f-1", "a step's output is visible");

    // `kill` writes only the request flag; the engine owns run status.
    ok(&["workflows", "kill", "report"]);
    let body = sql("SELECT VALUE kill_requested FROM ONLY _00_workflow_run:wf1;");
    assert!(body.contains("true"), "kill was not requested: {body}");
    let body = sql("SELECT VALUE status FROM ONLY _00_workflow_run:wf1;");
    assert!(body.contains("running"), "the CLI must not write run status itself: {body}");

    // `watch` without a TTY degrades to the static view rather than hanging.
    let text = ok(&["workflows", "watch", "report"]);
    assert!(text.contains("transform"), "watch fell back to show:\n{text}");
}
