//! End-to-end sync-pipeline heartbeat.
//!
//! Every cycle the probe UPSERTs `_00_heartbeat:probe` on the UPSTREAM
//! SurrealDB with a fresh `hb_seq`. The row's hand-written mutation event
//! (see `apps/cli/src/sp00ky.rs`) posts to this scheduler's `/ingest`, which
//! WALs and broadcasts it to every ready SSP, whose ingest handler records
//! the seq after stepping the circuit. The probe then polls each SSP's
//! `GET /debug/heartbeat` until ALL of them report the new seq (queries are
//! pinned per-SSP, so one deaf SSP is a real outage for its clients) and
//! records the wall-clock as the end-to-end latency.
//!
//! This exercises exactly the path that died in the 2026-08-08 outage:
//! DB event → scheduler HTTP → WAL → broadcast → SSP circuit. A wedged
//! scheduler stops the loop entirely — which is why the success-side ping
//! (dead-man's-switch, see `maintenance::alert`) is the primary alert
//! channel, and the failure webhook only the fast path.
//!
//! Results land in three places:
//! - `HeartbeatStats` (atomics) → `/metrics` + `/health` (staleness folds
//!   into "degraded"),
//! - `_00_heartbeat_rollup` hourly rows upstream (bounded time series the
//!   control plane can scrape),
//! - the alerter's ping/webhook URLs.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::router::SspPool;
use crate::transport::HttpTransport;
use maintenance::alert::Alerter;
use maintenance::db::ReconnectingDb;

/// Lock-free probe results, shared with the metrics/health handlers.
pub struct HeartbeatStats {
    /// Last successful cycle's end-to-end latency. `u64::MAX` = never.
    pub last_e2e_ms: AtomicU64,
    /// Epoch-ms of the last successful cycle. 0 = never.
    pub last_ok_epoch_ms: AtomicU64,
    /// Epoch-ms of the last attempt (any outcome). 0 = never.
    pub last_attempt_epoch_ms: AtomicU64,
    pub consecutive_failures: AtomicU32,
    /// False until `spawn` runs (and stays false when disabled via env), so
    /// `/health` doesn't report a stale heartbeat on deployments without one.
    pub enabled: AtomicBool,
}

impl HeartbeatStats {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            last_e2e_ms: AtomicU64::new(u64::MAX),
            last_ok_epoch_ms: AtomicU64::new(0),
            last_attempt_epoch_ms: AtomicU64::new(0),
            consecutive_failures: AtomicU32::new(0),
            enabled: AtomicBool::new(false),
        })
    }

    /// Staleness predicate for `/health`: enabled AND the last success is
    /// older than the grace window (3 full cycles + one timeout, or the
    /// `SPKY_HEALTH_MAX_HEARTBEAT_AGE_SECS` override if larger).
    pub fn is_stale(&self, cfg: &Config, now_epoch_ms: u64) -> bool {
        if !self.enabled.load(Ordering::Relaxed) {
            return false;
        }
        let last_ok = self.last_ok_epoch_ms.load(Ordering::Relaxed);
        let grace_secs = std::env::var("SPKY_HEALTH_MAX_HEARTBEAT_AGE_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
            .max(3 * cfg.interval_secs + cfg.timeout_secs);
        // Never-succeeded counts as stale once the grace window has passed
        // since boot would have allowed several cycles; use last_attempt as
        // the anchor when there has been no success yet.
        let anchor = if last_ok > 0 {
            last_ok
        } else {
            self.last_attempt_epoch_ms.load(Ordering::Relaxed)
        };
        anchor > 0 && now_epoch_ms.saturating_sub(anchor) > grace_secs * 1000
    }
}

#[derive(Clone)]
pub struct Config {
    /// 0 disables the probe entirely.
    pub interval_secs: u64,
    pub timeout_secs: u64,
    pub fail_threshold: u32,
    pub ping_url: Option<String>,
    pub webhook_url: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        let interval_secs: u64 = std::env::var("SPKY_HEARTBEAT_INTERVAL_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30);
        let timeout_secs = std::env::var("SPKY_HEARTBEAT_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n: &u64| *n > 0)
            .unwrap_or(25)
            // The cycle must finish before the next tick.
            .min(interval_secs.saturating_sub(1).max(1));
        Self {
            interval_secs,
            timeout_secs,
            fail_threshold: std::env::var("SPKY_HEARTBEAT_FAIL_THRESHOLD")
                .ok()
                .and_then(|s| s.parse().ok())
                .filter(|n: &u32| *n > 0)
                .unwrap_or(3),
            ping_url: std::env::var("SPKY_HEARTBEAT_PING_URL").ok(),
            webhook_url: std::env::var("SPKY_ALERT_WEBHOOK_URL").ok(),
        }
    }
}

enum CycleOutcome {
    Ok { e2e_ms: u64 },
    /// Nothing to probe (no ready SSPs). Counted, not alerted — `/health`
    /// already 503s on zero ready SSPs.
    Skipped(&'static str),
    Failed { stage: &'static str, detail: String },
}

fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Spawn the probe loop. No-op when `cfg.interval_secs == 0`.
pub fn spawn(
    db: Arc<ReconnectingDb>,
    ssp_pool: Arc<RwLock<SspPool>>,
    transport: Arc<HttpTransport>,
    stats: Arc<HeartbeatStats>,
    cfg: Config,
) {
    if cfg.interval_secs == 0 {
        info!("E2E heartbeat disabled (SPKY_HEARTBEAT_INTERVAL_SECS=0)");
        return;
    }
    stats.enabled.store(true, Ordering::Relaxed);
    info!(
        interval_secs = cfg.interval_secs,
        timeout_secs = cfg.timeout_secs,
        ping = cfg.ping_url.is_some(),
        webhook = cfg.webhook_url.is_some(),
        "E2E heartbeat probe started"
    );

    tokio::spawn(async move {
        let alerter = Alerter::new(
            cfg.ping_url.clone(),
            cfg.webhook_url.clone(),
            cfg.fail_threshold,
        );
        let mut interval = tokio::time::interval(Duration::from_secs(cfg.interval_secs));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last_prune_epoch_ms: u64 = 0;
        loop {
            interval.tick().await;
            let outcome = run_one_cycle(&db, &ssp_pool, &transport, cfg.timeout_secs).await;
            record(&stats, &alerter, &outcome);
            write_rollup(&db, &outcome).await;

            // Bounded retention: prune rollup rows older than 90 days, once
            // a day, from the probe loop itself (no reaper exists for this).
            let now = now_epoch_ms();
            if now.saturating_sub(last_prune_epoch_ms) > 24 * 3600 * 1000 {
                last_prune_epoch_ms = now;
                let handle = db.handle();
                if let Err(e) = handle
                    .query("DELETE _00_heartbeat_rollup WHERE bucket < time::now() - 90d")
                    .await
                {
                    debug!(error = %e, "heartbeat rollup prune failed");
                }
            }
        }
    });
}

/// One probe cycle: write upstream, then poll every ready SSP until all have
/// seen the new seq or the deadline passes.
async fn run_one_cycle(
    db: &Arc<ReconnectingDb>,
    ssp_pool: &Arc<RwLock<SspPool>>,
    transport: &Arc<HttpTransport>,
    timeout_secs: u64,
) -> CycleOutcome {
    // Snapshot (id, url) of ready SSPs — take-and-drop, never held across
    // an await (the whole point of this probe is catching lock convoys, not
    // causing them).
    let targets: Vec<(String, String)> = {
        let pool = ssp_pool.read().await;
        pool.all()
            .iter()
            .filter(|ssp| pool.is_ready(&ssp.id))
            .map(|ssp| (ssp.id.clone(), ssp.url.clone()))
            .collect()
    };
    if targets.is_empty() {
        return CycleOutcome::Skipped("no ready SSPs");
    }

    let hb_seq = now_epoch_ms();
    let started = tokio::time::Instant::now();
    let handle = db.handle();
    if let Err(e) = handle
        .query("UPSERT _00_heartbeat:probe SET hb_seq = $s")
        .bind(("s", hb_seq as i64))
        .await
    {
        let msg = e.to_string();
        db.note_error(&msg);
        return CycleOutcome::Failed {
            stage: "db_write",
            detail: msg,
        };
    }

    // Poll until every target reports hb_seq >= the seq we just wrote.
    let deadline = started + Duration::from_secs(timeout_secs);
    let mut pending: Vec<(String, String)> = targets;
    loop {
        let mut still_pending = Vec::new();
        for (id, url) in pending {
            let seen = match transport.get_from_ssp(&url, "/debug/heartbeat").await {
                Ok(resp) => resp
                    .json::<serde_json::Value>()
                    .await
                    .ok()
                    .and_then(|v| v.get("hb_seq").and_then(|s| s.as_u64())),
                Err(_) => None,
            };
            if seen.map(|s| s >= hb_seq).unwrap_or(false) {
                continue;
            }
            still_pending.push((id, url));
        }
        pending = still_pending;
        if pending.is_empty() {
            return CycleOutcome::Ok {
                e2e_ms: started.elapsed().as_millis() as u64,
            };
        }
        if tokio::time::Instant::now() >= deadline {
            let missing: Vec<&str> = pending.iter().map(|(id, _)| id.as_str()).collect();
            return CycleOutcome::Failed {
                stage: "ssp_visibility",
                detail: format!("SSPs never saw hb_seq {}: {}", hb_seq, missing.join(", ")),
            };
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn record(stats: &Arc<HeartbeatStats>, alerter: &Alerter, outcome: &CycleOutcome) {
    let now = now_epoch_ms();
    stats.last_attempt_epoch_ms.store(now, Ordering::Relaxed);
    match outcome {
        CycleOutcome::Ok { e2e_ms } => {
            stats.last_e2e_ms.store(*e2e_ms, Ordering::Relaxed);
            stats.last_ok_epoch_ms.store(now, Ordering::Relaxed);
            stats.consecutive_failures.store(0, Ordering::Relaxed);
            debug!(e2e_ms, "heartbeat ok");
            alerter.observe(true, 0, || {
                serde_json::json!({ "component": "scheduler_heartbeat", "e2e_ms": e2e_ms })
            });
        }
        CycleOutcome::Skipped(reason) => {
            // Not a failure of the pipeline itself; don't page, but do note.
            debug!(reason, "heartbeat skipped");
        }
        CycleOutcome::Failed { stage, detail } => {
            let failures = stats.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
            warn!(stage, %detail, failures, "heartbeat failed");
            alerter.observe(false, failures, || {
                serde_json::json!({
                    "component": "scheduler_heartbeat",
                    "stage": stage,
                    "detail": detail,
                    "consecutive_failures": failures,
                })
            });
        }
    }
}

/// Best-effort hourly rollup UPSERT upstream (`_00_heartbeat_rollup`).
/// The row id is the hour bucket, so concurrent schedulers can't race:
/// `count += 1` is atomic within SurrealDB.
async fn write_rollup(db: &Arc<ReconnectingDb>, outcome: &CycleOutcome) {
    let (ok, e2e_ms) = match outcome {
        CycleOutcome::Ok { e2e_ms } => (true, *e2e_ms),
        CycleOutcome::Failed { .. } => (false, 0),
        CycleOutcome::Skipped(_) => return,
    };
    let epoch_hour = now_epoch_ms() / 1000 / 3600 * 3600;
    let key = format!("h{}", epoch_hour);
    let query = if ok {
        "UPSERT type::thing('_00_heartbeat_rollup', $key) SET \
             bucket = <datetime>$bucket, count += 1, ok += 1, sum_ms += $ms, \
             max_ms = math::max([max_ms, $ms]), last_e2e_ms = $ms, last_ok_at = time::now()"
    } else {
        "UPSERT type::thing('_00_heartbeat_rollup', $key) SET \
             bucket = <datetime>$bucket, count += 1"
    };
    let bucket_iso = chrono::DateTime::from_timestamp(epoch_hour as i64, 0)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default();
    let handle = db.handle();
    if let Err(e) = handle
        .query(query)
        .bind(("key", key))
        .bind(("bucket", bucket_iso))
        .bind(("ms", e2e_ms as i64))
        .await
    {
        let msg = e.to_string();
        db.note_error(&msg);
        debug!(error = %msg, "heartbeat rollup write failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(interval: u64, timeout: u64) -> Config {
        Config {
            interval_secs: interval,
            timeout_secs: timeout,
            fail_threshold: 3,
            ping_url: None,
            webhook_url: None,
        }
    }

    #[test]
    fn staleness_needs_enabled_and_age_past_grace() {
        let c = cfg(30, 25);
        let stats = HeartbeatStats::new();
        let now: u64 = 10_000_000;

        // Disabled → never stale.
        stats.last_ok_epoch_ms.store(1, Ordering::Relaxed);
        assert!(!stats.is_stale(&c, now));

        stats.enabled.store(true, Ordering::Relaxed);
        // Grace = 3*30 + 25 = 115s. Fresh success → not stale.
        stats.last_ok_epoch_ms.store(now - 60_000, Ordering::Relaxed);
        assert!(!stats.is_stale(&c, now));
        // Older than grace → stale.
        stats.last_ok_epoch_ms.store(now - 200_000, Ordering::Relaxed);
        assert!(stats.is_stale(&c, now));

        // Never-succeeded anchors on last_attempt.
        let never = HeartbeatStats::new();
        never.enabled.store(true, Ordering::Relaxed);
        assert!(!never.is_stale(&c, now), "no attempts yet → not stale");
        never.last_attempt_epoch_ms.store(now - 200_000, Ordering::Relaxed);
        assert!(never.is_stale(&c, now));
    }

    #[test]
    fn outcome_recording_tracks_edges() {
        let stats = HeartbeatStats::new();
        let alerter = maintenance::alert::Alerter::new(None, None, 3);

        record(&stats, &alerter, &CycleOutcome::Failed {
            stage: "db_write",
            detail: "boom".into(),
        });
        record(&stats, &alerter, &CycleOutcome::Failed {
            stage: "db_write",
            detail: "boom".into(),
        });
        assert_eq!(stats.consecutive_failures.load(Ordering::Relaxed), 2);
        assert_eq!(stats.last_ok_epoch_ms.load(Ordering::Relaxed), 0);

        record(&stats, &alerter, &CycleOutcome::Ok { e2e_ms: 42 });
        assert_eq!(stats.consecutive_failures.load(Ordering::Relaxed), 0);
        assert_eq!(stats.last_e2e_ms.load(Ordering::Relaxed), 42);
        assert!(stats.last_ok_epoch_ms.load(Ordering::Relaxed) > 0);

        // Skipped cycles touch only last_attempt.
        let before = stats.last_e2e_ms.load(Ordering::Relaxed);
        record(&stats, &alerter, &CycleOutcome::Skipped("no ready SSPs"));
        assert_eq!(stats.last_e2e_ms.load(Ordering::Relaxed), before);
    }
}
