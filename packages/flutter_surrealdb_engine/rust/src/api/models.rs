
use flutter_rust_bridge::frb;

/// A strict representation of a SurrealDB Record ID.
/// Naming matches the standard `RecordId` expectation.
pub struct RecordId {
    pub tb: String,
    pub id: String,
}

impl RecordId {
    pub fn new(tb: String, id: String) -> Self {
        Self { tb, id }
    }

    #[frb(sync)]
    pub fn to_string(&self) -> String {
        format!("{}:{}", self.tb, self.id)
    }
}

