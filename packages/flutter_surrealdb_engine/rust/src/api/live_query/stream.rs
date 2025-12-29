use crate::api::live_query::models::{LiveQueryEvent, LiveQueryAction}; // Import Action
use crate::api::live_query::manager::LiveQueryManager;
use crate::frb_generated::StreamSink;
use futures_util::StreamExt;
use log::{info, error, warn};
use surrealdb::{Notification, Surreal};
use surrealdb::types::Value;
use serde_json::Value as JsonValue;

pub(crate) async fn run_live_query_loop<C: surrealdb::Connection>(
    db: Surreal<C>,
    table_name: String,
    sink: StreamSink<LiveQueryEvent>,
    query_uuid: String,
) {
    info!("Starting live query stream for: {} (UUID: {})", table_name, query_uuid);

    let resource = surrealdb::opt::Resource::Table(table_name.clone().into());
    let mut stream = match db.select(resource).live().await {
        Ok(s) => s,
        Err(e) => {
            let _ = sink.add_error(anyhow::anyhow!("Init error: {}", e));
            LiveQueryManager::unregister(&query_uuid);
            return;
        }
    };

    info!("LiveQuery: Stream running: {}", table_name);

    loop {
        let next_item: Option<surrealdb::Result<Notification<Value>>> = stream.next().await;
        match next_item {
            Some(Ok(notification)) => {
                log::debug!("RUST: Received notification: {:?}", notification);
                if let Err(e) = process_and_send(&notification, &sink) {
                    error!("Sink error, stopping: {}", e);
                    break;
                }
            },
            Some(Err(e)) => warn!("Stream error: {}", e),
            None => break,
        }
    }
    
    info!("LiveQuery ended: {}", table_name);
    LiveQueryManager::unregister(&query_uuid);
}

pub(crate) fn process_and_send(
    notification: &Notification<Value>, 
    sink: &StreamSink<LiveQueryEvent>
) -> anyhow::Result<()> {
    
    // Mapping: Surreal Action -> Unser Enum (Viel sauberer als String-Vergleich)
    let action_str = format!("{:?}", notification.action);
    let action = match action_str.as_str() {
        "Create" => LiveQueryAction::Create,
        "Update" => LiveQueryAction::Update,
        "Delete" => LiveQueryAction::Delete,
        _ => LiveQueryAction::Unknown,
    };

    // Convert Surreal Value to generic JSON Value for easier handling & serialization
    let json_data: JsonValue = notification.data.clone().into_json_value();

    let id = json_data.get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Wir senden die Daten als String, damit Dart sie flexibel parsen kann
    let event = LiveQueryEvent {
        action,
        result: json_data.to_string(), 
        id,
        query_uuid: None,
    };
    
    if sink.add(event).is_err() {
        return Err(anyhow::anyhow!("Sink closed"));
    }

    Ok(())
}