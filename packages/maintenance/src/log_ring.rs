//! In-process log ring + live tail.
//!
//! The scheduler and the SSP both log through `tracing` to stdout, which is
//! fine for `docker logs` and useless to anyone holding only an HTTP endpoint.
//! This adds a second sink: a bounded ring of recent lines plus a broadcast
//! channel, so `/admin/api/logs` can hand a client the recent history and then
//! keep it current without either side touching the filesystem or a log
//! shipper.
//!
//! Two properties this type must never lose:
//!
//! * **It cannot block the logger.** `tracing` events are emitted from inside
//!   arbitrary code, including latency-critical paths and code holding other
//!   locks. Every operation here is a `std::sync::Mutex` held for a handful of
//!   instructions with no await inside, and a `broadcast::send` (which never
//!   waits on a slow receiver — it drops for them instead).
//! * **A slow reader degrades only itself.** `broadcast` gives a lagging
//!   subscriber `RecvError::Lagged`, which the HTTP layer turns into a visible
//!   "N lines dropped" marker rather than a silent gap or a stalled writer.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::Context as LayerContext;
use tracing_subscriber::Layer;

/// One formatted log line, as served to a dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLine {
    /// Epoch milliseconds.
    pub ts: u64,
    /// `ERROR` / `WARN` / `INFO` / `DEBUG` / `TRACE`.
    pub level: String,
    /// The event's `tracing` target, e.g. `scheduler::ingest`.
    pub target: String,
    /// The event's `message` field.
    pub message: String,
    /// Remaining structured fields, rendered `key=value`, space separated.
    /// Empty when the event carried nothing but a message.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fields: String,
}

/// Bounded history plus a live feed.
pub struct LogRing {
    buf: Mutex<VecDeque<LogLine>>,
    tx: broadcast::Sender<LogLine>,
    capacity: usize,
}

impl LogRing {
    /// `capacity` lines of history. The broadcast channel is sized
    /// independently and much smaller: it only has to cover the gap between a
    /// burst and a subscriber's next poll, and an over-large one just delays
    /// the honest `Lagged` signal.
    pub fn new(capacity: usize) -> Arc<Self> {
        let (tx, _rx) = broadcast::channel(1024);
        Arc::new(Self {
            buf: Mutex::new(VecDeque::with_capacity(capacity.min(1024))),
            tx,
            capacity,
        })
    }

    /// Most recent `limit` lines, oldest first.
    pub fn snapshot(&self, limit: usize) -> Vec<LogLine> {
        let buf = self.buf.lock().expect("log ring poisoned");
        let skip = buf.len().saturating_sub(limit);
        buf.iter().skip(skip).cloned().collect()
    }

    /// Subscribe to lines emitted from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<LogLine> {
        self.tx.subscribe()
    }

    fn push(&self, line: LogLine) {
        {
            let mut buf = self.buf.lock().expect("log ring poisoned");
            if buf.len() == self.capacity {
                buf.pop_front();
            }
            buf.push_back(line.clone());
        }
        // Err only means "nobody is listening", which is the normal case.
        let _ = self.tx.send(line);
    }
}

/// Collects an event's fields, keeping `message` apart from the rest.
#[derive(Default)]
struct LineVisitor {
    message: String,
    fields: String,
}

impl LineVisitor {
    fn record(&mut self, field: &Field, value: String) {
        if field.name() == "message" {
            self.message = value;
        } else {
            if !self.fields.is_empty() {
                self.fields.push(' ');
            }
            self.fields.push_str(field.name());
            self.fields.push('=');
            self.fields.push_str(&value);
        }
    }
}

impl Visit for LineVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.record(field, format!("{:?}", value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record(field, value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record(field, value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record(field, value.to_string());
    }
}

/// The `tracing` layer that feeds a [`LogRing`]. Compose it alongside the
/// existing `fmt` layer so stdout keeps working exactly as before.
pub struct LogRingLayer {
    ring: Arc<LogRing>,
}

impl LogRingLayer {
    pub fn new(ring: Arc<LogRing>) -> Self {
        Self { ring }
    }
}

impl<S: tracing::Subscriber> Layer<S> for LogRingLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: LayerContext<'_, S>) {
        let mut visitor = LineVisitor::default();
        event.record(&mut visitor);

        let meta = event.metadata();
        self.ring.push(LogLine {
            ts: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            level: meta.level().to_string(),
            target: meta.target().to_string(),
            message: visitor.message,
            fields: visitor.fields,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(msg: &str) -> LogLine {
        LogLine {
            ts: 0,
            level: "INFO".into(),
            target: "t".into(),
            message: msg.into(),
            fields: String::new(),
        }
    }

    #[test]
    fn ring_evicts_oldest_at_capacity() {
        let ring = LogRing::new(3);
        for m in ["a", "b", "c", "d"] {
            ring.push(line(m));
        }
        let got: Vec<String> = ring.snapshot(10).into_iter().map(|l| l.message).collect();
        assert_eq!(got, vec!["b", "c", "d"]);
    }

    #[test]
    fn snapshot_limit_returns_the_newest() {
        let ring = LogRing::new(10);
        for m in ["a", "b", "c"] {
            ring.push(line(m));
        }
        let got: Vec<String> = ring.snapshot(2).into_iter().map(|l| l.message).collect();
        assert_eq!(got, vec!["b", "c"]);
    }

    #[test]
    fn subscribers_see_lines_pushed_after_subscribing() {
        let ring = LogRing::new(10);
        ring.push(line("before"));
        let mut rx = ring.subscribe();
        ring.push(line("after"));
        assert_eq!(rx.try_recv().unwrap().message, "after");
        assert!(rx.try_recv().is_err());
    }
}
