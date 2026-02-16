use anyhow::{Context, Result};
use redb::{Database, ReadableTable, ReadableTableMetadata, TableDefinition};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use surrealdb::Surreal;
use tracing::{debug, info};

// Re-export RecordOp from messages to avoid duplication
pub use crate::messages::RecordOp;

/// Stored record with version metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredRecord {
    record_id: String,
    current_version: u64,
    data: Value,
    updated_at: u64,
}

/// Versioned record for history
#[derive(Debug, Clone, Serialize, Deserialize)]
struct VersionedRecord {
    record_id: String,
    version: u64,
    data: Option<Value>,  // None for deletes
    updated_at: u64,
    operation: RecordOp,
}

/// Metadata for a table
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TableMetadata {
    table_name: String,
    record_count: usize,
    last_updated: u64,
}

/// Chunk of replica data for bootstrap
#[derive(Debug, Clone)]
pub struct ReplicaChunk {
    pub chunk_index: usize,
    pub table: String,
    pub records: Vec<(String, Value)>,
}

// Define static table definitions
const RECORDS_THREAD: TableDefinition<&str, &[u8]> = TableDefinition::new("records_thread");
const RECORDS_JOB: TableDefinition<&str, &[u8]> = TableDefinition::new("records_job");
const RECORDS_USER: TableDefinition<&str, &[u8]> = TableDefinition::new("records_user");

const VERSIONS_THREAD: TableDefinition<&str, &[u8]> = TableDefinition::new("versions_thread");
const VERSIONS_JOB: TableDefinition<&str, &[u8]> = TableDefinition::new("versions_job");
const VERSIONS_USER: TableDefinition<&str, &[u8]> = TableDefinition::new("versions_user");

const METADATA: TableDefinition<&str, &[u8]> = TableDefinition::new("_metadata");

/// Persistent replica backed by redb
pub struct Replica {
    db: Database,
    db_path: PathBuf,
    keep_versions: u64,
}

impl Replica {
    /// Create a new replica with persistent storage
    pub fn new(db_path: PathBuf, keep_versions: u64) -> Result<Self> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {:?}", parent))?;
        }

        let db = Database::create(&db_path)
            .with_context(|| format!("Failed to create/open redb at {:?}", db_path))?;

        info!("Opened replica database at {:?}", db_path);

        Ok(Self {
            db,
            db_path,
            keep_versions,
        })
    }

    /// Get table definition by name
    fn get_records_table_def(table_name: &str) -> &'static TableDefinition<'static, &'static str, &'static [u8]> {
        match table_name {
            "thread" => &RECORDS_THREAD,
            "job" => &RECORDS_JOB,
            "user" => &RECORDS_USER,
            _ => panic!("Unknown table: {}", table_name),
        }
    }

    /// Get versions table definition by name
    fn get_versions_table_def(table_name: &str) -> &'static TableDefinition<'static, &'static str, &'static [u8]> {
        match table_name {
            "thread" => &VERSIONS_THREAD,
            "job" => &VERSIONS_JOB,
            "user" => &VERSIONS_USER,
            _ => panic!("Unknown table: {}", table_name),
        }
    }

    /// Full initial load from SurrealDB
    pub async fn ingest_all<C>(&mut self, db: &Surreal<C>) -> Result<()>
    where
        C: surrealdb::Connection,
    {
        // Get list of tables
        let tables = vec!["thread", "job", "user"];

        for table_name in tables {
            info!("Ingesting table '{}'...", table_name);
            
            let records: Vec<Value> = db
                .select(table_name)
                .await
                .with_context(|| format!("Failed to select from table '{}'", table_name))?;

            // Use a write transaction for the whole table
            let write_txn = self.db.begin_write()?;
            {
                let table_def = Self::get_records_table_def(table_name);
                let mut table = write_txn.open_table(*table_def)?;

                for record in records {
                    if let Some(id) = record.get("id") {
                        let id_str = id.to_string();
                        let stored = StoredRecord {
                            record_id: id_str.clone(),
                            current_version: 1,
                            data: record.clone(),
                            updated_at: Self::now(),
                        };

                        let serialized = serde_json::to_vec(&stored)?;
                        table.insert(id_str.as_str(), serialized.as_slice())?;

                        // Also store initial version
                        self.store_version_raw(&write_txn, table_name, &id_str, 1, Some(record), RecordOp::Create)?;
                    }
                }

                // Update metadata
                let record_count = table.len()? as usize;
                self.update_metadata_raw(&write_txn, table_name, record_count)?;
            }
            write_txn.commit()?;

            info!("Ingested {} records from '{}'", self.get_table_count(table_name)?, table_name);
        }

        Ok(())
    }

    /// Apply a single record event
    pub fn apply(&mut self, table: &str, op: RecordOp, id: &str, record: Option<Value>) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let table_def = Self::get_records_table_def(table);
            let mut records_table = write_txn.open_table(*table_def)?;

            // Get current version
            let current_version = if let Some(existing) = records_table.get(id)? {
                let stored: StoredRecord = serde_json::from_slice(existing.value())?;
                stored.current_version
            } else {
                0
            };

            let new_version = current_version + 1;

            match op {
                RecordOp::Create | RecordOp::Update => {
                    if let Some(ref data) = record {
                        let stored = StoredRecord {
                            record_id: id.to_string(),
                            current_version: new_version,
                            data: data.clone(),
                            updated_at: Self::now(),
                        };

                        let serialized = serde_json::to_vec(&stored)?;
                        records_table.insert(id, serialized.as_slice())?;
                    }
                }
                RecordOp::Delete => {
                    records_table.remove(id)?;
                }
            }

            // Store version history
            self.store_version_raw(&write_txn, table, id, new_version, record, op)?;

            // Prune old versions if needed
            if self.keep_versions > 0 && new_version > self.keep_versions {
                self.prune_old_versions_raw(&write_txn, table, id, new_version)?;
            }

            // Update metadata
            let record_count = records_table.len()? as usize;
            self.update_metadata_raw(&write_txn, table, record_count)?;
        }
        write_txn.commit()?;

        debug!("Applied {:?} for {}:{} at version {}", op, table, id, 
               self.get_current_version(table, id).unwrap_or(0));

        Ok(())
    }

    /// Get a specific record
    pub fn get_record(&self, table: &str, id: &str) -> Result<Option<Value>> {
        let read_txn = self.db.begin_read()?;
        let table_def = Self::get_records_table_def(table);
        let records_table = read_txn.open_table(*table_def)?;

        if let Some(value) = records_table.get(id)? {
            let stored: StoredRecord = serde_json::from_slice(value.value())?;
            Ok(Some(stored.data))
        } else {
            Ok(None)
        }
    }

    /// Get a specific version of a record
    pub fn get_version(&self, table: &str, id: &str, version: u64) -> Result<Option<VersionedRecord>> {
        let read_txn = self.db.begin_read()?;
        let table_def = Self::get_versions_table_def(table);
        let versions_table = read_txn.open_table(*table_def)?;

        let key = format!("{}::{}", id, version);
        if let Some(value) = versions_table.get(key.as_str())? {
            let versioned: VersionedRecord = serde_json::from_slice(value.value())?;
            Ok(Some(versioned))
        } else {
            Ok(None)
        }
    }

    /// Get current version number for a record
    pub fn get_current_version(&self, table: &str, id: &str) -> Result<u64> {
        let read_txn = self.db.begin_read()?;
        let table_def = Self::get_records_table_def(table);
        let records_table = read_txn.open_table(*table_def)?;

        if let Some(value) = records_table.get(id)? {
            let stored: StoredRecord = serde_json::from_slice(value.value())?;
            Ok(stored.current_version)
        } else {
            Ok(0)
        }
    }

    /// List all versions of a record
    pub fn list_versions(&self, table: &str, id: &str) -> Result<Vec<u64>> {
        let read_txn = self.db.begin_read()?;
        let table_def = Self::get_versions_table_def(table);
        let versions_table = read_txn.open_table(*table_def)?;

        let prefix = format!("{}::", id);
        let mut versions = Vec::new();

        let range = versions_table.range(prefix.as_str()..)?;
        for item in range {
            let (key, _) = item?;
            if let Some(version_str) = key.value().strip_prefix(&prefix) {
                if let Ok(version) = version_str.parse::<u64>() {
                    versions.push(version);
                }
            }
        }

        Ok(versions)
    }

    /// Serialize all records for SSP bootstrap (chunked iterator)
    pub fn iter_chunks(&self, chunk_size: usize) -> Result<Vec<ReplicaChunk>> {
        let tables = vec!["thread", "job", "user"];
        let mut chunks = Vec::new();
        let mut chunk_index = 0;

        for table_name in tables {
            let read_txn = self.db.begin_read()?;
            let table_def = Self::get_records_table_def(table_name);
            let records_table = read_txn.open_table(*table_def)?;

            let mut current_chunk = Vec::new();

            for item in records_table.iter()? {
                let (key, value) = item?;
                let stored: StoredRecord = serde_json::from_slice(value.value())?;
                current_chunk.push((key.value().to_string(), stored.data));

                if current_chunk.len() >= chunk_size {
                    chunks.push(ReplicaChunk {
                        chunk_index,
                        table: table_name.to_string(),
                        records: std::mem::take(&mut current_chunk),
                    });
                    chunk_index += 1;
                }
            }

            // Push remaining records
            if !current_chunk.is_empty() {
                chunks.push(ReplicaChunk {
                    chunk_index,
                    table: table_name.to_string(),
                    records: current_chunk,
                });
                chunk_index += 1;
            }
        }

        Ok(chunks)
    }

    /// Get total record count across all tables
    pub fn record_count(&self) -> Result<usize> {
        let tables = vec!["thread", "job", "user"];
        let mut total = 0;

        for table_name in tables {
            total += self.get_table_count(table_name)?;
        }

        Ok(total)
    }

    /// Get record count for a specific table
    pub fn get_table_count(&self, table: &str) -> Result<usize> {
        let read_txn = self.db.begin_read()?;
        let table_def = Self::get_records_table_def(table);
        let records_table = read_txn.open_table(*table_def)?;
        Ok(records_table.len()? as usize)
    }

    /// Get number of tables
    pub fn table_count(&self) -> usize {
        3
    }

    // Helper methods

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn store_version_raw(
        &self,
        txn: &redb::WriteTransaction,
        table_name: &str,
        record_id: &str,
        version: u64,
        data: Option<Value>,
        operation: RecordOp,
    ) -> Result<()> {
        let table_def = Self::get_versions_table_def(table_name);
        let mut versions_table = txn.open_table(*table_def)?;

        let versioned = VersionedRecord {
            record_id: record_id.to_string(),
            version,
            data,
            updated_at: Self::now(),
            operation,
        };

        let key = format!("{}::{}", record_id, version);
        let serialized = serde_json::to_vec(&versioned)?;
        versions_table.insert(key.as_str(), serialized.as_slice())?;

        Ok(())
    }

    fn prune_old_versions_raw(
        &self,
        txn: &redb::WriteTransaction,
        table_name: &str,
        record_id: &str,
        current_version: u64,
    ) -> Result<()> {
        let table_def = Self::get_versions_table_def(table_name);
        let mut versions_table = txn.open_table(*table_def)?;

        let cutoff_version = current_version.saturating_sub(self.keep_versions);
        for version in 1..=cutoff_version {
            let key = format!("{}::{}", record_id, version);
            versions_table.remove(key.as_str())?;
        }

        Ok(())
    }

    fn update_metadata_raw(
        &self,
        txn: &redb::WriteTransaction,
        table_name: &str,
        record_count: usize,
    ) -> Result<()> {
        let metadata = TableMetadata {
            table_name: table_name.to_string(),
            record_count,
            last_updated: Self::now(),
        };

        let mut metadata_table = txn.open_table(METADATA)?;
        let serialized = serde_json::to_vec(&metadata)?;
        metadata_table.insert(table_name, serialized.as_slice())?;

        Ok(())
    }
}
