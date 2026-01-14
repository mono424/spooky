pub mod models;
pub mod manager;
pub mod stream;

use crate::api::client::SurrealDb;
use crate::frb_generated::StreamSink;
use crate::api::live_query::models::LiveQueryEvent;
use crate::api::live_query::manager::LiveQueryManager;
use crate::api::live_query::stream::run_live_query_loop;
use uuid::Uuid;
use log::info;

// Extending SurrealDb with live query capabilities
impl SurrealDb {
    pub async fn live_query(
        &self,
        table_name: String,
        sink: StreamSink<LiveQueryEvent>,
    ) -> anyhow::Result<()> {
        // Need to match on client type to pass correct concrete type to generic run loop
        let client = self.get_client().await?;
        let query_uuid = Uuid::new_v4().to_string();
        
        info!("LiveQuery: Starting for table '{}' with UUID: {}", table_name, query_uuid);

        // 1. Send Initial Handshake
        let handshake = LiveQueryEvent::handshake(query_uuid.clone());
        let _ = sink.add(handshake);

        // 2. Spawn Task
        let query_uuid_clone = query_uuid.clone();
        
        // Clone table_name for capture
        let table_name_clone = table_name.clone();

        let handle = tokio::spawn(async move {
            run_live_query_loop(client, table_name_clone, sink, query_uuid_clone).await;
        });

        // 3. Register Handle for Kill Switch
        LiveQueryManager::register(query_uuid, handle.abort_handle());

        Ok(())
    }

    pub fn kill_query(query_uuid: String) -> anyhow::Result<()> {
        LiveQueryManager::kill(&query_uuid)
    }
}
