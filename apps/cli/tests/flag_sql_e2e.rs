//! End-to-end test for `spky flag set --sql`.
//!
//! Drives the real `spky` binary against a throwaway SurrealDB started in
//! Docker: seeds a `user` table, assigns users to a flag by SQL, and asserts
//! the resolved allowlist rule and the materialized `_00_user_feature` rows.
//!
//! Marked `#[ignore]` so the default `cargo test` stays fast and CI-safe; run it
//! explicitly when Docker is available:
//!
//! ```sh
//! cargo test -p sp00ky-cli --test flag_sql_e2e -- --ignored
//! ```
//!
//! If Docker is not available the test skips (returns early) rather than failing,
//! mirroring the spooky_core integration tests.

use std::process::{Command, Output, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

const IMAGE: &str = "surrealdb/surrealdb:v2.1.4";
const NAME: &str = "spky-flag-sql-e2e";
const PORT: &str = "18077";
const AUTH: &str = "Basic cm9vdDpyb290"; // base64("root:root")
const NS: &str = "e2e";
const DB: &str = "e2e";

fn docker_available() -> bool {
    Command::new("docker")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Force-remove the container, ignoring errors (no-op if it isn't there).
fn remove_container() {
    let _ = Command::new("docker")
        .args(["rm", "-f", NAME])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// RAII guard so the container is torn down even if an assertion panics.
struct Container;
impl Drop for Container {
    fn drop(&mut self) {
        remove_container();
    }
}

/// POST a query to SurrealDB's HTTP `/sql`. Returns `None` on a transport error
/// (server not up yet), `Some(body)` otherwise (including HTTP error bodies).
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
    try_sql(query).expect("SurrealDB query failed (transport error)")
}

/// Run the built `spky` binary with the connection flags appended. stdin is
/// `/dev/null` so the run is non-interactive (no TTY).
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

#[test]
#[ignore = "requires Docker; run with: cargo test -p sp00ky-cli --test flag_sql_e2e -- --ignored"]
fn flag_set_sql_assigns_matched_users() {
    if !docker_available() {
        eprintln!("skipping flag_sql_e2e: Docker is not available");
        return;
    }

    remove_container(); // clear any stale container from a previous run
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
    let _guard = Container;

    // Wait for readiness (first run on a clean machine may pull the image).
    let deadline = Instant::now() + Duration::from_secs(90);
    while try_sql("RETURN 1;").is_none() {
        assert!(
            Instant::now() < deadline,
            "SurrealDB did not become ready within 90s"
        );
        sleep(Duration::from_millis(500));
    }

    // Seed: 3 adults (age > 18) and 2 non-adults.
    sql(
        "CREATE user:adult1 SET age = 25; CREATE user:adult2 SET age = 40; \
         CREATE user:adult3 SET age = 19; CREATE user:kid1 SET age = 10; \
         CREATE user:kid2 SET age = 18;",
    );

    // Create the flag (materializes every user to the default 'off').
    let out = spky(&["flag", "create", "demo"]);
    assert!(
        out.status.success(),
        "flag create failed: {}",
        combined(&out)
    );

    // Assign by SQL: the preview must report the matched count, then apply.
    let out = spky(&[
        "flag",
        "set",
        "demo",
        "--variant",
        "on",
        "--sql",
        "SELECT id FROM user WHERE age > 18",
        "--yes",
    ]);
    assert!(
        out.status.success(),
        "flag set --sql failed: {}",
        combined(&out)
    );
    assert!(
        combined(&out).contains("Matched 3 user(s)"),
        "expected a 3-user preview, got: {}",
        combined(&out)
    );

    // The three adults are 'on'; the two kids are not.
    let on = sql("SELECT VALUE user FROM _00_user_feature \
         WHERE key = 'demo' AND variant = 'on' ORDER BY user;");
    for u in ["user:adult1", "user:adult2", "user:adult3"] {
        assert!(on.contains(u), "expected {u} to be 'on': {on}");
    }
    for u in ["user:kid1", "user:kid2"] {
        assert!(!on.contains(u), "{u} must not be 'on': {on}");
    }

    // The two kids remain 'off'.
    let off = sql("SELECT VALUE user FROM _00_user_feature \
         WHERE key = 'demo' AND variant = 'off' ORDER BY user;");
    for u in ["user:kid1", "user:kid2"] {
        assert!(off.contains(u), "expected {u} to be 'off': {off}");
    }

    // The assignment is stored as a durable allowlist rule (so it survives the
    // re-materialization that other rule changes trigger), not a one-shot write.
    let rule = sql("SELECT VALUE rules FROM _00_feature_flag WHERE key = 'demo';");
    assert!(
        rule.contains(r#""kind":"allowlist""#),
        "rule not an allowlist: {rule}"
    );
    assert!(
        rule.contains(r#""variant":"on""#),
        "rule variant wrong: {rule}"
    );
    for u in ["user:adult1", "user:adult2", "user:adult3"] {
        assert!(rule.contains(u), "rule missing {u}: {rule}");
    }

    // A non-interactive run without --yes must refuse rather than apply blindly.
    let out = spky(&[
        "flag",
        "set",
        "demo",
        "--variant",
        "on",
        "--sql",
        "SELECT id FROM user WHERE age > 0",
    ]);
    assert!(!out.status.success(), "expected refusal without --yes");
    assert!(
        combined(&out).contains("pass --yes"),
        "expected a --yes hint, got: {}",
        combined(&out)
    );

    // A query that matches nobody reports it and writes nothing.
    let out = spky(&[
        "flag",
        "set",
        "demo",
        "--variant",
        "on",
        "--sql",
        "SELECT id FROM user WHERE age > 999",
        "--yes",
    ]);
    assert!(
        combined(&out).contains("No users matched"),
        "expected a no-match message, got: {}",
        combined(&out)
    );

    // Non-SELECT input is rejected before anything runs.
    let out = spky(&[
        "flag",
        "set",
        "demo",
        "--variant",
        "on",
        "--sql",
        "DELETE user",
        "--yes",
    ]);
    assert!(
        combined(&out).contains("must be a SELECT"),
        "expected a non-SELECT rejection, got: {}",
        combined(&out)
    );
}
