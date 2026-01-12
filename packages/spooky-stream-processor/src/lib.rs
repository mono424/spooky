// src/lib.rs

#[cfg(not(target_arch = "wasm32"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod converter;
pub mod engine; // <--- Das ist wichtig für den Test
pub mod sanitizer;
pub mod service;

#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
pub use rayon::prelude::*;

// Falls du noch StreamProcessor Traits hast, müssen die auch hier sein:
pub use engine::circuit::Circuit;
pub use engine::view::MaterializedViewUpdate;
pub use engine::view::QueryPlan;
use serde_json::Value;

pub trait StreamProcessor: Send + Sync {
    fn ingest_record(
        &mut self,
        table: String,
        op: String,
        id: String,
        record: Value,
        hash: String,
    ) -> Vec<MaterializedViewUpdate>;

    fn ingest_batch(
        &mut self,
        batch: Vec<(String, String, String, Value, String)>,
    ) -> Vec<MaterializedViewUpdate>;

    fn register_view(
        &mut self,
        plan: QueryPlan,
        params: Option<Value>,
    ) -> Option<MaterializedViewUpdate>;

    fn unregister_view(&mut self, id: &str);

    // Zero-Copy Enty Point
    fn ingest_bytes(&mut self, bytes: &[u8]) -> Vec<MaterializedViewUpdate> {
        // Use rkyv to access the archive without full deserialization (where possible)
        // or deserialize efficiently.
        // For simplicity: Deserialize to IngestBatch (which is standard deserialize).
        // True Zero-Copy access requires using `check_archived_root` and then reading fields.
        
        // use rkyv::Deserialize;
        
        // 1. Validation (Safe mode)
        let archived = rkyv::check_archived_root::<crate::engine::packet::IngestBatch>(bytes)
            .expect("Invalid data packet");
            
        // 2. Process
        // We need to convert ArchivedIngestPacket -> SpookyValue (via JSON parse of record_json)
        // This is where "Zero Copy I/O" ends and "Application Logic" begins.
        // We still parse JSON, but we saved parsing the outer envelope (table, op, id).
        
        let batch: Vec<(String, String, String, Value, String)> = archived.packets.iter().map(|pkt| {
            let table = pkt.table.as_str().to_string(); // rkyv String -> std Str -> String
            let op = pkt.op.as_str().to_string();
            let id = pkt.id.as_str().to_string();
            let hash = pkt.hash.as_str().to_string();
            
            let json_str = pkt.record_json.as_str();
            let val = serde_json::from_str(json_str).unwrap_or(Value::Null);
            
            (table, op, id, val, hash)
        }).collect();
        
        self.ingest_batch(batch)
    }
}
