use dashmap::DashMap;
use std::sync::OnceLock;
use tokio::task::AbortHandle;
use log::{info, warn};

/// Global registry for active live query tasks.
static LIVE_QUERY_HANDLES: OnceLock<DashMap<String, AbortHandle>> = OnceLock::new();

pub struct LiveQueryManager {}

impl LiveQueryManager {
    fn registry() -> &'static DashMap<String, AbortHandle> {
        LIVE_QUERY_HANDLES.get_or_init(|| DashMap::new())
    }

    /// Registers a new live query task with its UUID.
    pub(crate) fn register(uuid: String, handle: AbortHandle) {
        info!("LiveQueryRegistry: Registering task {}", uuid);
        Self::registry().insert(uuid, handle);
    }

    /// Unregisters a live query task (e.g., when it finishes normally).
    pub(crate) fn unregister(uuid: &str) {
        if Self::registry().remove(uuid).is_some() {
            info!("LiveQueryRegistry: Unregistered task {}", uuid);
        }
    }

    /// Kills a live query task by one-time abort.
    pub fn kill(uuid: &str) -> anyhow::Result<()> {
        if let Some((_, handle)) = Self::registry().remove(uuid) {
             info!("LiveQueryRegistry: Killing task {}", uuid);
             handle.abort();
             Ok(())
        } else {
            warn!("LiveQueryRegistry: Task {} not found (already stopped?)", uuid);
            Ok(())
        }
    }
}
