use serde_json::{json, Value};
use spooky_stream_processor::{
    Circuit, MaterializedViewUpdate, ViewUpdate,
};
// use uuid::Uuid; // Removed uuid
use ulid::Ulid;

pub fn setup() -> Circuit {
    Circuit::new()
}

pub fn generate_id() -> String {
    Ulid::new().to_string()
    // Using ulid because uuid was removed
}

pub fn generate_hash(record: &Value) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(record.to_string().as_bytes());
    hasher.finalize().to_hex().to_string()
}

pub fn ingest(
    circuit: &mut Circuit,
    table: &str,
    op: &str,
    id: &str,
    record: Value,
) -> Vec<MaterializedViewUpdate> {
    let hash = generate_hash(&record);
    println!("[Ingest] {} -> {}: {:#}", op, table, record);
    let updates = circuit.ingest_record(
        table,
        op,
        id,
        record,
        &hash,
        true, // is_optimistic = true for tests
    );
    // Convert ViewUpdate to MaterializedViewUpdate
    updates.into_iter().filter_map(unwrap_flat_update).collect()
}

/// Helper to extract MaterializedViewUpdate from ViewUpdate enum
pub fn unwrap_flat_update(update: ViewUpdate) -> Option<MaterializedViewUpdate> {
    match update {
        ViewUpdate::Flat(flat) | ViewUpdate::Tree(flat) => Some(flat),
        ViewUpdate::Streaming(_) => None, // Tests expect Flat format
    }
}

/// Helper trait to access common fields from ViewUpdate enum variants
pub trait ViewUpdateExt {
    fn query_id(&self) -> &str;
    fn result_data(&self) -> &[(String, u64)];
}

impl ViewUpdateExt for ViewUpdate {
    fn query_id(&self) -> &str {
        match self {
            ViewUpdate::Flat(flat) | ViewUpdate::Tree(flat) => &flat.query_id,
            ViewUpdate::Streaming(stream) => &stream.view_id,
        }
    }
    
    fn result_data(&self) -> &[(String, u64)] {
        match self {
            ViewUpdate::Flat(flat) | ViewUpdate::Tree(flat) => &flat.result_data,
            ViewUpdate::Streaming(_) => &[], // Streaming has delta events, not full snapshot
        }
    }
}


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

pub fn create_author(circuit: &mut Circuit, name: &str) -> String {
    let (id, record) = make_author_record(name);
    ingest(circuit, "author", "CREATE", &id, record);
    id
}

pub fn create_thread(circuit: &mut Circuit, title: &str, author_id: &str) -> String {
    let (id, record) = make_thread_record(title, author_id);
    ingest(circuit, "thread", "CREATE", &id, record);
    id
}

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
