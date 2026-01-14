use serde::{Deserialize, Serialize};

// Dieses Enum wird in Dart zu: enum LiveQueryAction { create, update, delete, unknown }
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum LiveQueryAction {
    Create,
    Update,
    Delete,
    Snapshot,
    Unknown,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct LiveQueryEvent {
    pub action: LiveQueryAction, // Typ-sicher statt String
    pub result: String,          // JSON String (leichter für Dart via jsonDecode)
    pub id: Option<String>,
    pub query_uuid: Option<String>,
}

impl LiveQueryEvent {
    // Handshake Helper
    pub fn handshake(query_uuid: String) -> Self {
        Self {
            action: LiveQueryAction::Unknown,
            result: "{}".to_string(),
            id: None,
            query_uuid: Some(query_uuid),
        }
    }
}