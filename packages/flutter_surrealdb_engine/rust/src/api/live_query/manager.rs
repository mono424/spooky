use dashmap::DashMap;
use once_cell::sync::Lazy;
use tokio::task::AbortHandle;
use log::{info, warn};

/// Global registry for active live query tasks.
static LIVE_QUERY_HANDLES: Lazy<DashMap<String, AbortHandle>> = Lazy::new(|| DashMap::new());

pub struct LiveQueryManager {}

impl LiveQueryManager {
    /// Registers a new live query task with its UUID.
    pub(crate) fn register(uuid: String, handle: AbortHandle) {
        info!("LiveQueryRegistry: Registering task {}", uuid);
        LIVE_QUERY_HANDLES.insert(uuid, handle);
    }

    /// Unregisters a live query task (e.g., when it finishes normally).
    pub(crate) fn unregister(uuid: &str) {
        if LIVE_QUERY_HANDLES.remove(uuid).is_some() {
            info!("LiveQueryRegistry: Unregistered task {}", uuid);
        }
    }

    /// Kills a live query task by one-time abort.
    pub fn kill(uuid: &str) -> anyhow::Result<()> {
        if let Some((_, handle)) = LIVE_QUERY_HANDLES.remove(uuid) {
             info!("LiveQueryRegistry: Killing task {}", uuid);
             handle.abort();
             Ok(())
        } else {
            warn!("LiveQueryRegistry: Task {} not found (already stopped?)", uuid);
            Ok(())
        }
    }
}
