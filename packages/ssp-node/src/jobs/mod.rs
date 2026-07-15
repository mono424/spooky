//! Job execution engine (absorbed from the former `packages/job-runner`).
//!
//! Fully portable: dispatch goes through the [`crate::ports::HttpClient`]
//! port, status writes through [`crate::ports::Db`], retry backoff through
//! [`crate::ports::Scheduler::sleep`] on a [`crate::ports::Spawner`] task.
//! The queue itself is `tokio::sync::mpsc` (wasm-safe); cancellation uses
//! the portable [`crate::ports::CancelHandle`]/[`CancelWatch`] pair instead
//! of `tokio_util::CancellationToken`.

mod runner;
mod types;

#[cfg(test)]
mod db_tests;

pub use runner::{
    append_error_helper, enqueue_recovered, fail_if_pending_helper, load_job_record,
    rearm_recurring_helper, reset_for_retry_helper, set_assignee_helper, update_status_helper,
    JobRunner, PENDING_DUE_CLAUSE, POKE_DUE_SQL,
};
pub use types::{BackendInfo, JobConfig, JobControl, JobEntry};
