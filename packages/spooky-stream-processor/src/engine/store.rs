use serde_json::Value;

/// Abstract interface for a data store that the Stream Processor can query.
/// This allows us to decouple the processor from the specific storage backend (SurrealDB, HashMap, etc.)
pub trait Store {
    /// Fetch a record by table and ID.
    fn get(&self, table: &str, id: &str) -> Option<Value>;

    /// Fetch records by a specific field value (for Joins).
    /// Note: The backend must support indexing on this field for performance.
    fn get_by_field(&self, table: &str, field: &str, value: &Value) -> Vec<Value>;
}
