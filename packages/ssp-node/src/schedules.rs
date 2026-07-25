//! Singlenode adapters for the shared scheduling engine.
//!
//! `schedule-core` owns all the logic; this file only supplies the two
//! capabilities it needs, in their standalone form:
//!
//! - [`PortDb`] forwards to the node's existing [`crate::ports::Db`] port, so the
//!   engine's SQL travels the same connection as everything else.
//! - [`InProcessJobKill`] cancels through the node's [`JobControl`], because in
//!   standalone mode this process is the only one that could be running the job.
//!   (The cluster scheduler broadcasts `/job/kill` instead — same trait, different
//!   adapter, one engine.)
//!
//! Only standalone nodes construct an engine. In cluster mode the scheduler
//! service is the single ticker; an SSP that also ticked would double-fire (the
//! engine's claim CAS would catch it, but the wasted work is pointless).

use std::sync::Arc;

use schedule_core::{EngineConfig, JobKill, ScheduleDb, ScheduleDbError, ScheduleEngine};
use serde_json::json;

use crate::jobs::{fail_if_pending_helper, JobControl};
use crate::ports::{Db, DbError};

/// Sweep cadence. Fast enough that `spky schedules trigger` feels immediate and
/// that a lost job-completion event heals quickly; slow enough to be free.
pub const SCHEDULE_SWEEP_INTERVAL_SECS: u64 = 5;

/// `ScheduleDb` over the node's `Db` port.
pub struct PortDb(pub Arc<dyn Db>);

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl ScheduleDb for PortDb {
    async fn query(
        &self,
        surql: &str,
        binds: &[(&str, serde_json::Value)],
    ) -> Result<Vec<serde_json::Value>, ScheduleDbError> {
        self.0.query(surql, binds).await.map_err(|e| match e {
            DbError::Auth(m) => ScheduleDbError::Auth(m),
            DbError::Transport(m) => ScheduleDbError::Transport(m),
            DbError::Query(m) => ScheduleDbError::Query(m),
        })
    }
}

/// Kill a job the way `/job/kill` does, minus the HTTP hop.
///
/// Mirrors `job_kill_handler`'s three cases: cancel an in-flight request, mark a
/// queued job killed so the runner drops it at dequeue, and terminalize a row
/// that was never enqueued at all. The runner stays the only writer of a
/// `processing` job's status.
pub struct InProcessJobKill {
    pub db: Arc<dyn Db>,
    pub job_control: JobControl,
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl JobKill for InProcessJobKill {
    async fn kill(&self, job_id: &str) -> anyhow::Result<()> {
        if self.job_control.cancel_inflight(job_id) {
            return Ok(());
        }
        // Not in flight: it may be sitting in the queue (flag it for dequeue) or
        // never have been enqueued (terminalize it under a `pending` guard).
        self.job_control.mark_killed_pending(job_id);
        let error = json!({ "code": "killed", "reason": "replaced by a newer scheduled run" });
        fail_if_pending_helper(self.db.as_ref(), job_id, error).await?;
        Ok(())
    }
}

/// Build the standalone engine. Returns `None` in cluster mode, where the
/// scheduler service owns ticking.
pub fn build_engine(
    standalone: bool,
    db: Arc<dyn Db>,
    job_control: JobControl,
) -> Option<Arc<ScheduleEngine>> {
    if !standalone {
        return None;
    }
    let kill = InProcessJobKill { db: Arc::clone(&db), job_control };
    Some(Arc::new(ScheduleEngine::new(
        Arc::new(PortDb(db)),
        Arc::new(kill),
        EngineConfig::default(),
    )))
}
