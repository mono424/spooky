use flutter_rust_bridge::frb;
use futures_util::StreamExt;
use surrealdb::engine::any::Any;
use surrealdb::{Surreal, Notification};
use crate::api::live_query::models::{LiveQueryEvent, LiveQueryAction};
use crate::frb_generated::StreamSink;
use crate::api::live_query::stream::process_and_send;

/// Connects to a Live Query, ensuring no data loss by starting the stream BEFORE fetching the snapshot.
/// Returns Ok(()) and streams events (Snapshot -> Handshake -> Updates) to `sink`.
pub(crate) async fn connect_live_query(
    db: Surreal<Any>,
    table: String,
    sink: StreamSink<LiveQueryEvent>,
) -> anyhow::Result<()> {
    // 1. Start the Live Query Stream FIRST (Buffering events)
    // This ensures we don't miss any events that happen while we are fetching the snapshot.
    // The stream is established on the table.
    let mut stream = db.select(&table).live().await?;
    
    // 2. Fetch the Snapshot (Initial Data)
    let snapshot_data: Vec<surrealdb::types::Value> = db.select(&table).await?;
    
    // Convert snapshot to clean JSON
    let snapshot_clean: Vec<serde_json::Value> = snapshot_data
        .into_iter()
        .map(|v| v.into_json_value())
        .collect();

    let snapshot_json = serde_json::to_string(&snapshot_clean)?;

    // Send Snapshot Event immediately
    let snapshot_event = LiveQueryEvent {
        action: LiveQueryAction::Snapshot,
        result: snapshot_json,
        id: None,
        // We use a temporary UUID or try to get it from stream if possible.
        // The real UUID comes from the handshake event from the stream typically.
        query_uuid: Some("snapshot".to_string()), 
    };
    
    if let Err(e) = sink.add(snapshot_event) {
        return Err(anyhow::anyhow!("Failed to send snapshot: {}", e));
    }
    
    // 3. Spawning the Stream Handler Task
    tokio::spawn(async move {
        // StreamExt trait must be in scope for .next() to work on Stream
        use futures_util::StreamExt;
        
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(notification) => {
                    // Reuse the robust processing logic from stream.rs
                    if let Err(e) = process_and_send(&notification, &sink) {
                        log::error!("StreamSink error: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    log::error!("Live Query Stream error: {}", e);
                    break;
                }
            }
        }
    });

    Ok(())
}
