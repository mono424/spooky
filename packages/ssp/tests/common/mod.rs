//! Common utilities for spooky-stream-processor benchmarks
//!
//! Provides setup functions, ID generation, hashing, and record creation helpers
//! that match the sync engine's usage patterns.

use serde_json::{json, Value};
use ssp::{
    engine::update::{ViewResultFormat, ViewUpdate},
    Circuit,
};
use ulid::Ulid;
use smol_str::SmolStr;

/// Extension trait to provide convenient accessors for ViewUpdate
pub trait ViewUpdateExt {
    fn result_data(&self) -> &[SmolStr];
    fn query_id(&self) -> &str;
}

impl ViewUpdateExt for ViewUpdate {
    fn result_data(&self) -> &[SmolStr] {
        match self {
            ViewUpdate::Flat(m) | ViewUpdate::Tree(m) => &m.result_data,
            ViewUpdate::Streaming(_) => &[],
        }
    }

    fn query_id(&self) -> &str {
        match self {
            ViewUpdate::Flat(m) | ViewUpdate::Tree(m) => &m.query_id,
            ViewUpdate::Streaming(s) => &s.view_id,
        }
    }
}

/// Create a new Circuit instance for benchmarking
pub fn setup() -> Circuit {
    Circuit::new()
}

/// Generate a unique ID using ULID (matches sync engine pattern)
pub fn generate_id() -> String {
    Ulid::new().to_string()
}

/// Generate a blake3 hash for a record (matches sync engine hashing)
pub fn generate_hash(record: &Value) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(record.to_string().as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Ingest a single record and return view updates
/// This mirrors the sync engine's `ingest_handler` flow
pub fn ingest(
    circuit: &mut Circuit,
    table: &str,
    op: &str,
    id: &str,
    record: Value,
) -> Vec<ViewUpdate> {
    use ssp::engine::circuit::dto::BatchEntry;
    use ssp::engine::types::Operation;

    let operation = Operation::from_str(op).expect("Invalid operation string");
    let entry = BatchEntry::new(table, operation, id, record.into());
    
    // ingest_single returns SmallVec, convert to Vec for compatibility
    circuit.ingest_single(entry).into_vec()
}

/// Ingest with verbose logging (useful for debugging)
pub fn ingest_verbose(
    circuit: &mut Circuit,
    table: &str,
    op: &str,
    id: &str,
    record: Value,
) -> Vec<ViewUpdate> {
    println!("[Ingest] {} -> {}: {:#}", op, table, record);
    ingest(circuit, table, op, id, record)
}

/// Create an author record (matches sync engine data model)
pub fn make_author_record(name: &str) -> (String, Value) {
    let id_raw = generate_id();
    let id = format!("author:{}", id_raw);
    let record = json!({
        "id": id,
        "name": name,
        "type": "author"
    });
    (id, record)
}

/// Create a thread record (matches sync engine data model)
pub fn make_thread_record(title: &str, author_id: &str) -> (String, Value) {
    let id_raw = generate_id();
    let id = format!("thread:{}", id_raw);
    let record = json!({
        "id": id,
        "title": title,
        "author": author_id,
        "type": "thread"
    });
    (id, record)
}

/// Create a comment record (matches sync engine data model)
pub fn make_comment_record(text: &str, thread_id: &str, author_id: &str) -> (String, Value) {
    let id_raw = generate_id();
    let id = format!("comment:{}", id_raw);
    let record = json!({
        "id": id,
        "text": text,
        "thread": thread_id,
        "author": author_id,
        "type": "comment"
    });
    (id, record)
}

/// Create and ingest an author, returning the author ID
pub fn create_author(circuit: &mut Circuit, name: &str) -> String {
    let (id, record) = make_author_record(name);
    ingest(circuit, "author", "CREATE", &id, record);
    id
}

/// Create and ingest a thread, returning the thread ID
pub fn create_thread(circuit: &mut Circuit, title: &str, author_id: &str) -> String {
    let (id, record) = make_thread_record(title, author_id);
    ingest(circuit, "thread", "CREATE", &id, record);
    id
}

/// Create and ingest a comment, returning the comment ID
pub fn create_comment(
    circuit: &mut Circuit,
    text: &str,
    thread_id: &str,
    author_id: &str,
) -> String {
    let (id, record) = make_comment_record(text, thread_id, author_id);
    ingest(circuit, "comment", "CREATE", &id, record);
    id
}

/// Create an author with a specific format for view registration
pub fn create_author_with_format(
    circuit: &mut Circuit,
    name: &str,
    _format: ViewResultFormat,
) -> String {
    // Note: format only affects views, not record ingestion
    create_author(circuit, name)
}

/// Batch ingest helper for high-throughput scenarios
pub fn ingest_batch(
    circuit: &mut Circuit,
    records: Vec<(String, String, String, Value)>,
) -> Vec<ViewUpdate> {
    use ssp::engine::circuit::dto::BatchEntry;
    use ssp::engine::types::Operation;

    let batch: Vec<BatchEntry> = records
        .into_iter()
        .map(|(table, op, id, record)| {
            let operation = Operation::from_str(&op).expect("Invalid operation string in test");
            BatchEntry::new(table, operation, id, record.into())
        })
        .collect();

    circuit.ingest_batch(batch)
}

/// Helper to count updates by type
#[derive(Debug, Default)]
pub struct UpdateStats {
    pub flat_updates: usize,
    pub tree_updates: usize,
    pub streaming_updates: usize,
    pub total_records: usize,
}

impl UpdateStats {
    pub fn count(updates: &[ViewUpdate]) -> Self {
        let mut stats = Self::default();
        for update in updates {
            match update {
                ViewUpdate::Flat(m) => {
                    stats.flat_updates += 1;
                    stats.total_records += m.result_data.len();
                }
                ViewUpdate::Tree(m) => {
                    stats.tree_updates += 1;
                    stats.total_records += m.result_data.len();
                }
                ViewUpdate::Streaming(s) => {
                    stats.streaming_updates += 1;
                    stats.total_records += s.records.len();
                }
            }
        }
        stats
    }
}