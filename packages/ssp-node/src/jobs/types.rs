use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::ports::{CancelHandle, CancelWatch};

/// Shared control state for in-flight cancellation and kill-while-pending.
///
/// Constructed once by the shell and cloned into both the [`super::JobRunner`]
/// and the `/job/kill` + `/job/retry` HTTP handlers, so both sides see the
/// same maps. Cheap to clone (one `Arc`).
///
/// wasm-safe by construction: a plain `std::sync::Mutex` over the maps (never
/// held across an await; uncontended on a single-threaded DO) and the portable
/// watch-based cancel pair instead of `DashMap` + `tokio_util::CancellationToken`.
#[derive(Clone, Default)]
pub struct JobControl(Arc<Mutex<ControlInner>>);

#[derive(Default)]
struct ControlInner {
    /// `job_id` -> cancel handle for the request currently in-flight on this
    /// node. Registered by the runner just before the HTTP POST and released
    /// right after. A kill handler fires the handle; the runner's cancellable
    /// `HttpClient::send` observes it.
    inflight: HashMap<String, CancelHandle>,
    /// `job_id`s that were killed while still queued (not yet in-flight). The
    /// runner checks this set at dequeue time and fails the job instead of
    /// running it. This is the synchronization point that avoids a second
    /// writer racing the runner on the `status` field.
    killed_pending: HashSet<String>,
    /// `job_id`s currently represented in the in-memory queue on this node —
    /// queued or in-flight, but not yet terminal. Inserted at every enqueue
    /// and removed when the job reaches a terminal state. The recovery sweep
    /// consults this so it never re-enqueues a job that is already moving
    /// through the runner (which would double-execute it).
    enqueued: HashSet<String>,
}

impl JobControl {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a job as queued. Returns `true` if it was newly marked (the caller
    /// should send it to the runner), `false` if it was already queued/in-flight
    /// (the caller must skip the send to avoid a double execution).
    pub fn mark_enqueued(&self, id: &str) -> bool {
        self.0.lock().unwrap().enqueued.insert(id.to_string())
    }

    /// Clear a job's queued mark once it reaches a terminal state, so a later
    /// (e.g. retried) life of the same id can be enqueued again.
    pub fn clear_enqueued(&self, id: &str) {
        self.0.lock().unwrap().enqueued.remove(id);
    }

    /// Register the in-flight request for `id`; the returned watch is passed
    /// to `HttpClient::send` so a kill can abort it. Runner-only.
    pub fn register_inflight(&self, id: &str) -> CancelWatch {
        let (handle, watch) = CancelHandle::new();
        self.0.lock().unwrap().inflight.insert(id.to_string(), handle);
        watch
    }

    /// Release the in-flight registration after the request resolves. The
    /// runner is the single consumer, so only one execution per job_id is
    /// ever in-flight; an unconditional remove cannot drop a different
    /// (e.g. retried) execution's handle.
    pub fn release_inflight(&self, id: &str) {
        self.0.lock().unwrap().inflight.remove(id);
    }

    /// Fire the cancel handle for an in-flight job. Returns `true` when the
    /// job was in-flight on this node (the handle was fired).
    pub fn cancel_inflight(&self, id: &str) -> bool {
        match self.0.lock().unwrap().inflight.get(id) {
            Some(handle) => {
                handle.cancel();
                true
            }
            None => false,
        }
    }

    pub fn is_inflight(&self, id: &str) -> bool {
        self.0.lock().unwrap().inflight.contains_key(id)
    }

    /// Flag a job killed before it went in-flight; the runner fails it at
    /// dequeue time instead of executing it.
    pub fn mark_killed_pending(&self, id: &str) {
        self.0.lock().unwrap().killed_pending.insert(id.to_string());
    }

    /// Consume the killed-while-pending flag. Returns `true` when the flag
    /// was set (the caller must fail the job without executing it).
    pub fn take_killed_pending(&self, id: &str) -> bool {
        self.0.lock().unwrap().killed_pending.remove(id)
    }
}

/// Info about a single backend that handles jobs
#[derive(Clone, Debug)]
pub struct BackendInfo {
    pub name: String,
    pub base_url: String,
    pub auth_token: Option<String>,
    pub timeout: Option<u32>,
    pub timeout_overridable: bool,
}

impl BackendInfo {
    /// Compute the effective timeout for a job, considering the backend default
    /// and an optional per-job override (only used if timeout_overridable is true).
    pub fn effective_timeout(&self, job_override: Option<u32>) -> Duration {
        let base = self.timeout.unwrap_or(10);
        let seconds = if self.timeout_overridable {
            job_override.unwrap_or(base)
        } else {
            base
        };
        Duration::from_secs(seconds as u64)
    }
}

/// Configuration mapping job tables to their backends
#[derive(Clone, Default, Debug)]
pub struct JobConfig {
    /// Maps table_name -> backend info
    pub job_tables: HashMap<String, BackendInfo>,
}

impl JobConfig {
    /// Parse the `SPKY_JOB_CONFIG` payload — a JSON array of
    /// `{name, table, base_url, auth_token, timeout, timeout_overridable}` —
    /// into a table→backend map. Shared by every host shell (VM env var, CF
    /// Worker binding). `null` / `[]` / unparseable ⇒ empty config (job runner
    /// disabled), never an error: an empty backend list is a legitimate state.
    pub fn from_json(json: &str) -> JobConfig {
        let entries: Vec<Value> = match serde_json::from_str::<Option<Vec<Value>>>(json) {
            Ok(Some(v)) => v,
            _ => return JobConfig::default(),
        };
        let mut job_tables = HashMap::new();
        for entry in &entries {
            let table = entry.get("table").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let base_url = entry.get("base_url").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if table.is_empty() || base_url.is_empty() {
                continue;
            }
            job_tables.insert(
                table,
                BackendInfo {
                    name: entry.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    base_url,
                    auth_token: entry.get("auth_token").and_then(|v| v.as_str()).map(String::from),
                    timeout: entry.get("timeout").and_then(|v| v.as_u64()).map(|v| v as u32),
                    timeout_overridable: entry
                        .get("timeout_overridable")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                },
            );
        }
        JobConfig { job_tables }
    }
}

/// A job entry in the queue (includes which backend to call)
#[derive(Clone, Debug)]
pub struct JobEntry {
    pub id: String,       // e.g., "job:abc123"
    pub base_url: String, // e.g., "http://localhost:3000"
    pub path: String,     // e.g., "/spookify"
    pub payload: Value,
    pub retries: u32,
    pub max_retries: u32,
    pub retry_strategy: String, // "linear" or "exponential"
    pub auth_token: Option<String>,
    pub timeout: Duration,
}

impl JobEntry {
    /// Create a JobEntry from record data
    pub fn from_record(
        id: String,
        base_url: String,
        auth_token: Option<String>,
        record: &Value,
        timeout: Duration,
    ) -> Self {
        Self {
            id,
            base_url,
            path: record
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            payload: record.get("payload").cloned().unwrap_or(Value::Null),
            retries: record.get("retries").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            max_retries: record
                .get("max_retries")
                .and_then(|v| v.as_u64())
                .unwrap_or(3) as u32,
            retry_strategy: record
                .get("retry_strategy")
                .and_then(|v| v.as_str())
                .unwrap_or("linear")
                .to_string(),
            auth_token,
            timeout,
        }
    }
}

#[cfg(test)]
mod job_config_from_json_tests {
    use super::JobConfig;

    #[test]
    fn parses_outbox_entries_and_tolerates_empty() {
        assert!(JobConfig::from_json("null").job_tables.is_empty());
        assert!(JobConfig::from_json("[]").job_tables.is_empty());
        assert!(JobConfig::from_json("not json").job_tables.is_empty());
        let c = JobConfig::from_json(
            r#"[{"name":"api","table":"job","base_url":"https://api.example.com","timeout":7}]"#,
        );
        let b = c.job_tables.get("job").expect("job table present");
        assert_eq!(b.name, "api");
        assert_eq!(b.base_url, "https://api.example.com");
        assert_eq!(b.timeout, Some(7));
        // entries missing table or base_url are skipped
        assert!(JobConfig::from_json(r#"[{"name":"x","table":"job"}]"#).job_tables.is_empty());
    }
}
