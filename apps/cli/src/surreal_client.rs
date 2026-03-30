use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct SurrealResponse {
    pub status: String,
    pub result: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppliedMigration {
    pub version: String,
    pub name: String,
    pub applied_at: String,
    pub checksum: String,
}

/// Trait abstracting the SurrealDB operations needed by the migration system.
/// This enables testing with a mock implementation.
pub trait MigrationDB {
    fn ping(&self) -> Result<()>;
    fn ensure_ns_db(&self) -> Result<()>;
    fn ensure_migration_table(&self) -> Result<()>;
    fn execute(&self, query: &str) -> Result<Vec<SurrealResponse>>;
    fn get_applied_migrations(&self) -> Result<Vec<AppliedMigration>>;
    fn record_migration(&self, version: &str, name: &str, checksum: &str) -> Result<()>;
    fn update_migration_checksum(&self, version: &str, new_checksum: &str) -> Result<()>;
}

pub struct SurrealClient {
    url: String,
    namespace: String,
    database: String,
    auth_header: String,
}

impl SurrealClient {
    pub fn new(
        url: &str,
        namespace: &str,
        database: &str,
        username: &str,
        password: &str,
    ) -> Self {
        let credentials = format!("{}:{}", username, password);
        let auth_header = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(credentials)
        );
        Self {
            url: url.trim_end_matches('/').to_string(),
            namespace: namespace.to_string(),
            database: database.to_string(),
            auth_header,
        }
    }

    /// Create a client without authentication (for unauthenticated SurrealDB instances).
    pub fn new_unauthenticated(url: &str, namespace: &str, database: &str) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            namespace: namespace.to_string(),
            database: database.to_string(),
            auth_header: String::new(),
        }
    }
}

/// Send a raw SQL query to SurrealDB via HTTP, returning parsed responses.
///
/// This is the shared helper for all ureq call sites. It extracts the response
/// body from HTTP errors (so we see the actual SurrealDB message) and checks
/// for ERR status in the parsed response.
fn send_raw_sql(
    url: &str,
    auth_header: &str,
    ns_header: Option<&str>,
    db_header: Option<&str>,
    query: &str,
) -> Result<Vec<SurrealResponse>> {
    let mut req = ureq::post(url)
        .set("Accept", "application/json");

    if !auth_header.is_empty() {
        req = req.set("Authorization", auth_header);
    }

    if let Some(ns) = ns_header {
        req = req.set("surreal-ns", ns);
    }
    if let Some(db) = db_header {
        req = req.set("surreal-db", db);
    }

    let resp = match req.send_string(query) {
        Ok(resp) => resp,
        Err(ureq::Error::Status(code, response)) => {
            let body = response.into_string().unwrap_or_default();
            bail!("SurrealDB returned HTTP {}: {}", code, body);
        }
        Err(ureq::Error::Transport(t)) => {
            bail!("Failed to connect to SurrealDB: {}", t);
        }
    };

    let body: Vec<SurrealResponse> = resp
        .into_json()
        .context("Failed to parse SurrealDB response")?;

    for r in &body {
        if r.status == "ERR" {
            let msg = r
                .result
                .as_ref()
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            bail!("SurrealDB error: {}", msg);
        }
    }

    Ok(body)
}

impl SurrealClient {
    /// Returns the result of INFO FOR DB as a JSON value.
    pub fn info_for_db(&self) -> Result<serde_json::Value> {
        let responses = self
            .execute_query("INFO FOR DB;")
            .context("Failed to execute INFO FOR DB")?;
        let result = responses
            .into_iter()
            .next()
            .and_then(|r| r.result)
            .unwrap_or(serde_json::Value::Null);
        Ok(result)
    }

    /// Returns the result of INFO FOR TABLE <name> as a JSON value.
    pub fn info_for_table(&self, table: &str) -> Result<serde_json::Value> {
        let query = format!("INFO FOR TABLE {};", table);
        let responses = self
            .execute_query(&query)
            .context(format!("Failed to execute INFO FOR TABLE {}", table))?;
        let result = responses
            .into_iter()
            .next()
            .and_then(|r| r.result)
            .unwrap_or(serde_json::Value::Null);
        Ok(result)
    }

    /// Internal execute that returns parsed responses (shared by trait impl and info methods).
    fn execute_query(&self, query: &str) -> Result<Vec<SurrealResponse>> {
        send_raw_sql(
            &format!("{}/sql", self.url),
            &self.auth_header,
            Some(&self.namespace),
            Some(&self.database),
            query,
        )
    }

    /// Drop and recreate the database (used to recover from stale migration state in dev).
    pub fn reset_database(&self) -> Result<()> {
        let query = format!(
            "USE NS {}; REMOVE DATABASE {};",
            self.namespace, self.database
        );
        send_raw_sql(
            &format!("{}/sql", self.url),
            &self.auth_header,
            None,
            None,
            &query,
        )
        .context("Failed to remove database")?;

        self.ensure_ns_db()
    }

    /// Ensure namespace and database exist (usable outside the MigrationDB trait).
    pub fn ensure_ns_db(&self) -> Result<()> {
        let query = format!(
            "DEFINE NAMESPACE IF NOT EXISTS {}; USE NS {}; DEFINE DATABASE IF NOT EXISTS {};",
            self.namespace, self.namespace, self.database
        );
        send_raw_sql(
            &format!("{}/sql", self.url),
            &self.auth_header,
            None,
            None,
            &query,
        )
        .context("Failed to create namespace/database")?;

        Ok(())
    }
}

impl MigrationDB for SurrealClient {
    fn ensure_ns_db(&self) -> Result<()> {
        // Delegate to the inherent method to avoid duplication.
        SurrealClient::ensure_ns_db(self)
    }

    fn execute(&self, query: &str) -> Result<Vec<SurrealResponse>> {
        self.execute_query(query)
    }

    fn ping(&self) -> Result<()> {
        self.execute("INFO FOR DB;")
            .context("Failed to ping SurrealDB")?;
        Ok(())
    }

    fn ensure_migration_table(&self) -> Result<()> {
        let schema = include_str!("migration_tables.surql");
        self.execute(schema)
            .context("Failed to create migration tracking table")?;
        Ok(())
    }

    fn get_applied_migrations(&self) -> Result<Vec<AppliedMigration>> {
        let responses = match self
            .execute("SELECT version, name, applied_at, checksum FROM _00_migrations ORDER BY version ASC;")
        {
            Ok(r) => r,
            Err(e) => {
                // SurrealDB 3.x returns error for non-existent tables — treat as empty
                let msg = e.to_string();
                if msg.contains("does not exist") || msg.contains("NotFound") {
                    return Ok(vec![]);
                }
                return Err(e).context("Failed to query applied migrations");
            }
        };

        let result = responses
            .into_iter()
            .next()
            .and_then(|r| r.result)
            .unwrap_or(serde_json::Value::Array(vec![]));

        let migrations: Vec<AppliedMigration> =
            serde_json::from_value(result).context("Failed to deserialize applied migrations")?;

        Ok(migrations)
    }

    fn record_migration(&self, version: &str, name: &str, checksum: &str) -> Result<()> {
        let query = format!(
            "CREATE _00_migrations SET version = '{}', name = '{}', checksum = '{}';",
            version, name, checksum
        );
        self.execute(&query)
            .context("Failed to record migration")?;
        Ok(())
    }

    fn update_migration_checksum(&self, version: &str, new_checksum: &str) -> Result<()> {
        let query = format!(
            "UPDATE _00_migrations SET checksum = '{}' WHERE version = '{}';",
            new_checksum, version
        );
        self.execute(&query)
            .context("Failed to update migration checksum")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_constructs_basic_auth_header() {
        let client = SurrealClient::new(
            "http://localhost:8000",
            "test_ns",
            "test_db",
            "root",
            "root",
        );
        // base64("root:root") = "cm9vdDpyb290"
        assert_eq!(client.auth_header, "Basic cm9vdDpyb290");
    }

    #[test]
    fn test_new_encodes_special_chars_in_credentials() {
        let client = SurrealClient::new(
            "http://localhost:8000",
            "ns",
            "db",
            "admin",
            "p@ss:w0rd!",
        );
        let expected = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("admin:p@ss:w0rd!")
        );
        assert_eq!(client.auth_header, expected);
    }

    #[test]
    fn test_new_trims_trailing_slash_from_url() {
        let client = SurrealClient::new(
            "http://localhost:8000/",
            "ns",
            "db",
            "root",
            "root",
        );
        assert_eq!(client.url, "http://localhost:8000");
    }

    #[test]
    fn test_new_trims_multiple_trailing_slashes() {
        let client = SurrealClient::new(
            "http://localhost:8000///",
            "ns",
            "db",
            "root",
            "root",
        );
        assert_eq!(client.url, "http://localhost:8000");
    }

    #[test]
    fn test_new_preserves_url_without_trailing_slash() {
        let client = SurrealClient::new(
            "http://localhost:8000",
            "ns",
            "db",
            "root",
            "root",
        );
        assert_eq!(client.url, "http://localhost:8000");
    }

    #[test]
    fn test_new_stores_namespace_and_database() {
        let client = SurrealClient::new(
            "http://localhost:8000",
            "my_namespace",
            "my_database",
            "root",
            "root",
        );
        assert_eq!(client.namespace, "my_namespace");
        assert_eq!(client.database, "my_database");
    }

    #[test]
    fn test_surreal_response_deserialize_ok() {
        let json = r#"{"status":"OK","result":[{"id":"test:1"}]}"#;
        let resp: SurrealResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, "OK");
        assert!(resp.result.is_some());
    }

    #[test]
    fn test_surreal_response_deserialize_err() {
        let json = r#"{"status":"ERR","result":"Some error message"}"#;
        let resp: SurrealResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, "ERR");
        assert_eq!(resp.result.unwrap().as_str().unwrap(), "Some error message");
    }

    #[test]
    fn test_surreal_response_deserialize_null_result() {
        let json = r#"{"status":"OK","result":null}"#;
        let resp: SurrealResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, "OK");
        assert!(resp.result.is_none());
    }

    #[test]
    fn test_applied_migration_deserialize() {
        let json = r#"{
            "version": "20240101120000",
            "name": "initial_schema",
            "applied_at": "2024-01-01T12:05:00Z",
            "checksum": "abc123"
        }"#;
        let m: AppliedMigration = serde_json::from_str(json).unwrap();
        assert_eq!(m.version, "20240101120000");
        assert_eq!(m.name, "initial_schema");
        assert_eq!(m.applied_at, "2024-01-01T12:05:00Z");
        assert_eq!(m.checksum, "abc123");
    }

    #[test]
    fn test_applied_migration_deserialize_from_array() {
        let json = r#"[
            {"version":"20240101120000","name":"first","applied_at":"2024-01-01T12:00:00Z","checksum":"aaa"},
            {"version":"20240102120000","name":"second","applied_at":"2024-01-02T12:00:00Z","checksum":"bbb"}
        ]"#;
        let migrations: Vec<AppliedMigration> = serde_json::from_str(json).unwrap();
        assert_eq!(migrations.len(), 2);
        assert_eq!(migrations[0].version, "20240101120000");
        assert_eq!(migrations[1].version, "20240102120000");
    }
}
