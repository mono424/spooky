//! Job-kill port.
//!
//! `concurrency: replace` and killing a workflow run both need to stop an
//! in-flight job, and how you do that differs per mode: singlenode cancels the
//! in-process `CancelHandle` directly, while the cluster scheduler broadcasts
//! `/job/kill` to every ready SSP because it doesn't know which one claimed the
//! row. The engine only needs "make this job stop".
//!
//! Kills are best-effort by nature (the request may already be mid-flight), so
//! every engine write that follows a kill is guarded by `WHERE status = ...` —
//! a late completion can never overwrite the terminal state the engine chose.

use crate::MaybeSendSync;

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait JobKill: MaybeSendSync {
    /// Stop the job with this outbox record id. Returning `Ok` means the kill
    /// was dispatched, not that the job had not already finished.
    async fn kill(&self, job_id: &str) -> anyhow::Result<()>;
}

/// No-op implementation for hosts that cannot kill (and for tests that only
/// exercise `concurrency: skip` / `allow`).
pub struct NoopJobKill;

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl JobKill for NoopJobKill {
    async fn kill(&self, job_id: &str) -> anyhow::Result<()> {
        tracing::debug!(job_id, "job kill requested but no kill capability is wired");
        Ok(())
    }
}
