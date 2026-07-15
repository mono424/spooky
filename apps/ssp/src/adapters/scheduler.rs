use std::collections::HashMap;
use std::sync::Mutex;

use ssp_node::{CancelHandle, Scheduler, TimerKind};
use tokio::sync::mpsc;

/// `ssp_node::Scheduler` on tokio: one sleeping task per pending timer,
/// fired kinds delivered through an mpsc channel the shell drains and
/// dispatches to the node. Replace-on-reschedule = cancel the previous
/// task for that kind before spawning a new one.
pub struct TokioScheduler {
    tx: mpsc::UnboundedSender<TimerKind>,
    pending: Mutex<HashMap<TimerKind, CancelHandle>>,
}

impl TokioScheduler {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<TimerKind>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { tx, pending: Mutex::new(HashMap::new()) }, rx)
    }
}

#[async_trait::async_trait]
impl Scheduler for TokioScheduler {
    async fn schedule(&self, kind: TimerKind, at_epoch_ms: u64) {
        let (handle, mut watch) = CancelHandle::new();
        if let Some(prev) = self.pending.lock().unwrap().insert(kind.clone(), handle) {
            prev.cancel();
        }

        let tx = self.tx.clone();
        let delay = at_epoch_ms.saturating_sub(ssp_node::now_epoch_ms());
        tokio::spawn(async move {
            tokio::select! {
                _ = watch.cancelled() => {} // replaced or cancelled
                _ = tokio::time::sleep(std::time::Duration::from_millis(delay)) => {
                    let _ = tx.send(kind);
                }
            }
        });
    }

    async fn cancel(&self, kind: &TimerKind) {
        if let Some(handle) = self.pending.lock().unwrap().remove(kind) {
            handle.cancel();
        }
    }

    async fn sleep(&self, dur: std::time::Duration) {
        tokio::time::sleep(dur).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fires_and_replace_on_reschedule() {
        let (sched, mut rx) = TokioScheduler::new();

        // Reschedule replaces: only the second deadline fires.
        sched.schedule(TimerKind::TtlCleanup, ssp_node::now_epoch_ms() + 5_000).await;
        sched.schedule(TimerKind::TtlCleanup, ssp_node::now_epoch_ms() + 10).await;
        let fired = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("timer should fire")
            .unwrap();
        assert_eq!(fired, TimerKind::TtlCleanup);

        // Cancel prevents firing.
        sched.schedule(TimerKind::EdgeFlush, ssp_node::now_epoch_ms() + 10).await;
        sched.cancel(&TimerKind::EdgeFlush).await;
        let nothing =
            tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await;
        assert!(nothing.is_err(), "cancelled timer must not fire");
    }
}
