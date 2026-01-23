use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Notify, RwLock};
use tokio::time::sleep;
use tracing::{info, error, debug};
use ssp::Circuit;
use crate::persistence;

pub struct BackgroundSaver {
    path: PathBuf,
    circuit: Arc<RwLock<Circuit>>,
    notify: Arc<Notify>,
    shutdown: Arc<Notify>,
    debounce_duration: Duration,
}

impl BackgroundSaver {
    pub fn new(path: PathBuf, circuit: Arc<RwLock<Circuit>>, debounce_ms: u64) -> Self {
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
                    
                    self.save_now().await;
                }
                _ = self.shutdown.notified() => {
                    info!("Shutdown signal received, performing final save");
                    self.save_now().await;
                    break;
                }
            }
        }
        info!("Background saver stopped");
    }

    pub fn signal_shutdown(&self) {
        self.shutdown.notify_one();
    }

    async fn save_now(&self) {
        debug!("Saving state to disk...");
        let circuit_guard = self.circuit.read().await;
        if let Err(e) = persistence::save_circuit(&self.path, &circuit_guard) {
            error!("Background save failed: {}", e);
        } else {
            debug!("Background save completed");
        }
    }
}

