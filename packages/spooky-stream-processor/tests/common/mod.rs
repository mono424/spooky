use serde_json::{json, Value};
use spooky_stream_processor::{Circuit, MaterializedViewUpdate};
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
    circuit.ingest_record(
        table.to_string(),
        op.to_string(),
        id.to_string(),
        record,
        hash,
    )
}

pub fn create_author(circuit: &mut Circuit, name: &str) -> String {
    let id_raw = generate_id();
    let id = format!("author:{}", id_raw);
    let record = json!({
        "id": id,
        "name": name,
        "type": "author"
    });
    ingest(circuit, "author", "CREATE", &id, record);
    id
}

pub fn create_thread(circuit: &mut Circuit, title: &str, author_id: &str) -> String {
    let id_raw = generate_id();
    let id = format!("thread:{}", id_raw);
    let record = json!({
        "id": id,
        "title": title,
        "author": author_id,
        "type": "thread"
    });
    ingest(circuit, "thread", "CREATE", &id, record);
    id
}

pub fn create_comment(
    circuit: &mut Circuit,
    text: &str,
    thread_id: &str,
    author_id: &str,
) -> String {
    let id_raw = generate_id();
    let id = format!("comment:{}", id_raw);
    let record = json!({
        "id": id,
        "text": text,
        "thread": thread_id,
        "author": author_id,
        "type": "comment"
    });
    ingest(circuit, "comment", "CREATE", &id, record);
    id
}
