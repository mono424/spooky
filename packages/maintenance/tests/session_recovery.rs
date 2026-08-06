//! Regression test for the outage where a SurrealDB restart permanently broke
//! every long-lived `Surreal<Http>` handle in the SSP and scheduler.
//!
//! The HTTP engine registers a session in SurrealDB's in-memory session map and
//! tags every later request with its UUID. A restart empties that map, so the
//! old handle fails forever with `Session not found: <uuid>` — and re-running
//! `signin` on it cannot help, because the signin is itself routed through the
//! dead session. Production ran 84 minutes with no jobs, no view registration
//! and no realtime until the containers were restarted by hand.
//!
//! This test reproduces that exact sequence against a real SurrealDB and
//! asserts both halves: that a plain handle stays broken, and that
//! `ReconnectingDb` recovers on its own.
//!
//! Requires Docker. Run with:
//!   cargo test -p maintenance --test session_recovery -- --ignored --nocapture

use maintenance::db::{connect_http, DbConfig, ReconnectingDb};

const IMAGE: &str = "surrealdb/surrealdb:v3.0.5";
const CONTAINER: &str = "spky-session-recovery-test";
const PORT: u16 = 18099;

fn docker(args: &[&str]) -> std::process::Output {
    std::process::Command::new("docker")
        .args(args)
        .output()
        .expect("docker not runnable")
}

fn config() -> DbConfig {
    DbConfig {
        url: format!("http://127.0.0.1:{PORT}"),
        namespace: "test".to_string(),
        database: "test".to_string(),
        username: "root".to_string(),
        password: "root".to_string(),
    }
}

async fn wait_until_serving() {
    let url = format!("http://127.0.0.1:{PORT}/health");
    for _ in 0..120 {
        if matches!(reqwest::get(&url).await, Ok(r) if r.status().is_success()) {
            // /health flips green a beat before the RPC endpoint settles.
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    panic!("SurrealDB never became healthy on port {PORT}");
}

#[tokio::test]
#[ignore = "requires docker"]
async fn reconnecting_db_survives_a_surrealdb_restart() {
    let _ = docker(&["rm", "-f", CONTAINER]);
    let run = docker(&[
        "run", "-d", "--name", CONTAINER,
        "-p", &format!("{PORT}:8000"),
        IMAGE,
        "start", "--user", "root", "--pass", "root", "--allow-all",
    ]);
    assert!(
        run.status.success(),
        "docker run failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    // Teardown runs on the success path; a panicking run leaves the container
    // behind, which the `docker rm -f` above cleans up on the next run.
    run_scenario().await;
    let _ = docker(&["rm", "-f", CONTAINER]);
}

async fn run_scenario() {
    wait_until_serving().await;
    let cfg = config();

    // A plain long-lived handle, exactly what the SSP and scheduler used to
    // hold, plus the self-healing wrapper that replaced it.
    let plain = connect_http(&cfg).await.expect("plain connect failed");
    let reconnecting = ReconnectingDb::connect(&cfg)
        .await
        .expect("reconnecting connect failed");

    plain.query("RETURN 1").await.expect("plain: healthy query failed");
    reconnecting
        .handle()
        .query("RETURN 1")
        .await
        .expect("reconnecting: healthy query failed");

    // The incident: SurrealDB restarts. Its session map is memory-only, so
    // every session attached by the handles above is gone.
    let restart = docker(&["restart", CONTAINER]);
    assert!(
        restart.status.success(),
        "docker restart failed: {}",
        String::from_utf8_lossy(&restart.stderr)
    );
    wait_until_serving().await;

    // Half 1 — the bug. The old handle is dead, and re-signing in on it does
    // NOT bring it back: the signin travels through the same dead session.
    let err = plain
        .query("RETURN 1")
        .await
        .expect_err("plain handle should be dead after a restart")
        .to_string();
    assert!(
        maintenance::db::is_dead_session_error(&err),
        "expected a dead-session error, got: {err}"
    );

    let resignin = plain
        .signin(surrealdb::opt::auth::Root {
            username: cfg.username.clone(),
            password: cfg.password.clone(),
        })
        .await;
    assert!(
        resignin.is_err(),
        "re-signin on a dead session unexpectedly succeeded — if the SDK gained \
         session recovery, ReconnectingDb can be simplified"
    );
    assert!(
        plain.query("RETURN 1").await.is_err(),
        "plain handle recovered on its own, which the outage proves it does not"
    );

    // Half 2 — the fix. One refresh pass notices the dead session and swaps in
    // a brand-new handle, so callers keep working without a process restart.
    assert!(
        reconnecting.refresh().await,
        "refresh should report the connection usable after reconnecting"
    );
    reconnecting
        .handle()
        .query("RETURN 1")
        .await
        .expect("reconnecting handle should work again after refresh");
}
