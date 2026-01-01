use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::sleep;
use tracing::{info, error, debug};
use spooky_stream_processor::Circuit;
use crate::persistence;

pub struct BackgroundSaver {
    path: PathBuf,
    circuit: Arc<Mutex<Box<Circuit>>>,
    notify: Arc<Notify>,
    shutdown: Arc<Notify>,
    debounce_duration: Duration,
}

impl BackgroundSaver {
    pub fn new(path: PathBuf, circuit: Arc<Mutex<Box<Circuit>>>, debounce_ms: u64) -> Self {
        Self {
            path,
            circuit,
            notify: Arc::new(Notify::new()),
            shutdown: Arc::new(Notify::new()),
            debounce_duration: Duration::from_millis(debounce_ms),
        }
    }

    pub fn trigger_save(&self) {
        self.notify.notify_one();
    }

    pub async fn run(self: Arc<Self>) {
        info!("Background saver started");
        loop {
            tokio::select! {
                _ = self.notify.notified() => {
                    // Debounce
                    debug!("Change detected, waiting {:?} before saving", self.debounce_duration);
                    sleep(self.debounce_duration).await;
                    
                    // Consume any extra notifications that happened during sleep
                    // access internal logic or just proceed. 
                    // Notify uses standard behavior, multiple notifications while not awaiting are coalesced? 
                    // No, `notify_one` wakes one waiter. If we are sleeping, we are not waiting.
                    // But `notified()` is cancellation safe. 
                    // Let's just save.
                    
                    self.save_now();
                }
                _ = self.shutdown.notified() => {
                    info!("Shutdown signal received, performing final save");
                    self.save_now();
                    break;
                }
            }
        }
        info!("Background saver stopped");
    }

    pub fn signal_shutdown(&self) {
        self.shutdown.notify_one();
    }

    fn save_now(&self) {
        debug!("Saving state to disk...");
        let circuit_guard = self.circuit.lock().unwrap();
        if let Err(e) = persistence::save_circuit(&self.path, &circuit_guard) {
            error!("Background save failed: {}", e);
        } else {
            debug!("Background save completed");
        }
    }
}
