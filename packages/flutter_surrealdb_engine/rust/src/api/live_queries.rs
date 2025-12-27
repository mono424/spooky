use crate::api::client::SurrealDb;
use crate::frb_generated::StreamSink;
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use surrealdb::Notification; 
use log::{info, error, warn};

#[derive(Serialize)]
pub(crate) struct LiveQueryEvent<'a> {
    action: &'a str, 
    result: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
}

impl SurrealDb {
    pub async fn live_query(
        &self,
        table_name: String,
        sink: StreamSink<String>,
    ) -> anyhow::Result<()> {
        let db = self.get_db()?;

        tokio::spawn(async move {
            info!("Starting live query stream for: {}", table_name);

            let mut stream = match db.select(&table_name).live().await {
                Ok(s) => s,
                Err(e) => {
                    let _ = sink.add_error(anyhow::anyhow!("Init error: {}", e));
                    return;
                }
            };

            while let Some(result) = stream.next().await {
                match result {
                    Ok(notification) => {
                        if let Err(e) = Self::process_and_send(&notification, &sink) {
                            error!("Failed to process/send live event: {}", e);
                        }
                    },
                    Err(e) => {
                        warn!("Stream error from SurrealDB: {}", e);
                    }
                }
            }
            
            info!("Live query stream ended for {}", table_name);
        });

        Ok(())
    }

    fn process_and_send(
        notification: &Notification<Value>, 
        sink: &StreamSink<String>
    ) -> anyhow::Result<()> {
        // Optimierung: String matching because Action enum is not easily accessible
        let action_s = notification.action.to_string().to_uppercase();
        let action_str = match action_s.as_str() {
            "CREATE" => "CREATE",
            "UPDATE" => "UPDATE",
            "DELETE" => "DELETE",
            _ => "UNKNOWN", 
        };

        // ID Extraktion
        let id = notification.data.get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let event = LiveQueryEvent {
            action: action_str,
            result: notification.data.clone(), 
            id,
        };

        let json_str = serde_json::to_string(&event)?;

        if sink.add(json_str).is_err() {
            return Err(anyhow::anyhow!("Sink closed by client"));
        }

        Ok(())
    }
}