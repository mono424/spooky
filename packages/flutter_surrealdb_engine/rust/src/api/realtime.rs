// use flutter_rust_bridge::frb;
use futures_util::StreamExt;
use surrealdb::engine::any::Any;
use surrealdb::Surreal;
use crate::api::live_query::models::{LiveQueryEvent, LiveQueryAction};
use crate::frb_generated::StreamSink;
use crate::api::live_query::stream::process_and_send;
use uuid::Uuid;

/// Connects to a Live Query, ensuring no data loss by starting the stream BEFORE fetching the snapshot.
/// Returns Ok(()) and streams events (Snapshot -> Handshake -> Updates) to `sink`.
// Pure Live Query moved to api/live_query/mod.rs (legacy implementation)

/// Snapshot + Live Query Stream
/// Used by `SurrealDb.select(tableName).live()`
pub(crate) async fn connect_live_query_with_snapshot(
    db: Surreal<Any>,
    table: String,
    sink: StreamSink<LiveQueryEvent>,
) -> anyhow::Result<()> {
    // 1. Fetch the Snapshot (Initial Data) - FIRST
    log::info!("RUST: [Snapshot] Fetching...");
    let resource_snapshot = surrealdb::opt::Resource::Table(table.clone().into());
    // Create clone for snapshot
    let db_snapshot = db.clone();
    
    let snapshot_clean: Vec<serde_json::Value> = match db_snapshot.select(resource_snapshot).await {
        Ok(val) => {
            log::info!("RUST: [Snapshot] Fetched Successfully.");
            match val {
                 surrealdb::types::Value::Array(arr) => arr.into_iter().map(|v| v.into_json_value()).collect(),
                 single => vec![single.into_json_value()],
            }
        },
        Err(e) => {
            log::error!("RUST ERROR: [Snapshot] Failed: {}. Proceeding empty.", e);
            vec![]
        }
    };

    let snapshot_json = serde_json::to_string(&snapshot_clean)?;
    let snapshot_event = LiveQueryEvent {
        action: LiveQueryAction::Snapshot,
        result: snapshot_json,
        id: None,
        query_uuid: Some("snapshot".to_string()), 
    };
    
    if let Err(e) = sink.add(snapshot_event) {
        log::error!("RUST ERROR: [Snapshot] Sink error: {}", e);
    }
    
    // 2. Spawn Task to Connect & Read Stream
    // We move the connection logic INSIDE the spawn to prevent blocking the FFI return.
    log::info!("RUST: [Bg Task] Spawning background task for table: {}", table);
    
    // Create clones for the async block
    let table_name_clone = table.clone();
    let db_stream = db.clone();
    
    tokio::spawn(async move {
        use futures_util::StreamExt;
        
        log::info!("RUST: [Bg Task] Task Started. Connecting to Live Query...");
        let resource_stream = surrealdb::opt::Resource::Table(table_name_clone.clone().into());
        
        let mut stream_result = db_stream.select(resource_stream).live().await;
        
        if let Err(e) = stream_result {
             log::error!("RUST ERROR: [Bg Task] Failed to connect live query: {}", e);
             return;
        }
        let mut stream = stream_result.unwrap();

        log::info!("RUST: [Bg Task] Connected! Starting listen loop...");
        while let Some(msg) = stream.next().await {
            // log::debug!("RUST: [Loop] Received Message!"); 
            match msg {
                Ok(notification) => {
                    // log::debug!("RUST: [Loop] Processing Notification: {:?}", notification.action);
                    if let Err(e) = process_and_send(&notification, &sink) {
                        log::error!("StreamSink error: {}", e);
                        log::error!("RUST ERROR: [Loop] Sink error: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    log::error!("Live Query Stream error: {}", e);
                    log::error!("RUST ERROR: [Loop] Stream error: {}", e); 
                    break;
                }
            }
        }
        log::info!("RUST: [Bg Loop] Exited.");
    });

    Ok(())
}
