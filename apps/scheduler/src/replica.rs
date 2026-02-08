use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Instant;
use surrealdb::Surreal;

/// In-memory replica of all SurrealDB records
pub struct Replica {
    /// table_name → record_id → record_value
    tables: HashMap<String, HashMap<String, Value>>,
    /// Tracks the last update timestamp for consistency
    last_updated: Instant,
}

impl Replica {
    /// Create a new empty replica
    pub fn new() -> Self {
        Self {
            tables: HashMap::new(),
            last_updated: Instant::now(),
        }
    }

    /// Full initial load from SurrealDB
    pub async fn ingest_all<C>(&mut self, db: &Surreal<C>) -> Result<()>
    where
        C: surrealdb::Connection,
    {
        // Get list of tables
        // For now, hardcoded - TODO: discover from schema
        let tables = vec!["thread", "job", "user"];

        for table in tables {
            let records: Vec<Value> = db.select(table).await
                .with_context(|| format!("Failed to select from table '{}'", table))?;

            let mut table_map = HashMap::new();
            for record in records {
                if let Some(id) = record.get("id") {
                    let id_str = id.to_string();
                    table_map.insert(id_str, record);
                }
            }

            self.tables.insert(table.to_string(), table_map);
        }

        self.last_updated = Instant::now();
        Ok(())
    }

    /// Apply a single record event
    pub fn apply(&mut self, table: &str, op: RecordOp, id: &str, record: Option<Value>) {
        match op {
            RecordOp::Create | RecordOp::Update => {
                if let Some(record) = record {
                    self.tables
                        .entry(table.to_string())
                        .or_insert_with(HashMap::new)
                        .insert(id.to_string(), record);
                }
            }
            RecordOp::Delete => {
                if let Some(table_map) = self.tables.get_mut(table) {
                    table_map.remove(id);
                }
            }
        }
        self.last_updated = Instant::now();
    }

    /// Serialize all records for SSP bootstrap (chunked iterator)
    pub fn iter_chunks(&self, chunk_size: usize) -> Vec<ReplicaChunk> {
        let mut chunks = Vec::new();
        let mut chunk_index = 0;

        for (table, records) in &self.tables {
            let mut current_chunk = Vec::new();

            for (id, record) in records {
                current_chunk.push((id.clone(), record.clone()));

                if current_chunk.len() >= chunk_size {
                    chunks.push(ReplicaChunk {
                        chunk_index,
                        table: table.clone(),
                        records: std::mem::take(&mut current_chunk),
                    });
                    chunk_index += 1;
                }
            }

            // Push remaining records
            if !current_chunk.is_empty() {
                chunks.push(ReplicaChunk {
                    chunk_index,
                    table: table.clone(),
                    records: current_chunk,
                });
                chunk_index += 1;
            }
        }

        chunks
    }

    /// Get record count for monitoring
    pub fn record_count(&self) -> usize {
        self.tables.values().map(|t| t.len()).sum()
    }

    /// Get table count
    pub fn table_count(&self) -> usize {
        self.tables.len()
    }
}

/// Record operation type
#[derive(Debug, Clone, Copy)]
pub enum RecordOp {
    Create,
    Update,
    Delete,
}

/// Chunk of replica data for bootstrap
#[derive(Debug, Clone)]
pub struct ReplicaChunk {
    pub chunk_index: usize,
    pub table: String,
    pub records: Vec<(String, Value)>,
}
