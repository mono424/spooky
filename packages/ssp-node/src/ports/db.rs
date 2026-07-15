use super::MaybeSendSync;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// Signin token lapsed or credentials rejected — the adapter should
    /// re-signin and retry once before surfacing this.
    #[error("auth: {0}")]
    Auth(String),
    /// Network / connection-level failure (retryable by the caller).
    #[error("transport: {0}")]
    Transport(String),
    /// The database executed the statement and returned an error.
    #[error("query: {0}")]
    Query(String),
}

/// Database access port.
///
/// Deliberately OUR abstraction rather than the `surrealdb` SDK: the SDK's
/// wasm32 story is an unverified spike (S1 in `docs/platform-architecture.md`),
/// and the core's usage is narrow enough that a hand-rolled HTTP `/sql`
/// adapter (~150 lines) is a drop-in fallback. Results use the flattened-JSON
/// convention of `surrealdb::types::Value::into_json_value()`: RecordIds and
/// Datetimes arrive as plain strings, no tagged enum shapes.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait Db: MaybeSendSync {
    /// Execute one SurrealQL statement (or a `;`-joined transaction block)
    /// with bindings. Returns one flattened-JSON value per statement.
    async fn query(
        &self,
        surql: &str,
        binds: &[(&str, serde_json::Value)],
    ) -> Result<Vec<serde_json::Value>, DbError>;

    /// Server version string (surfaced via `/info`).
    async fn version(&self) -> Result<String, DbError>;
}
