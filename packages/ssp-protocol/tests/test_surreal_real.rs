use ssp_protocol::snapshot_hash::hash_table;
use serde_json::{json, Value};
use std::fs;

#[test]
fn test_real_hash() {
    let json_data = fs::read_to_string("/tmp/all_games.json").unwrap();
    let rows: Vec<Value> = serde_json::from_str(&json_data).unwrap();

    // 1. Scheduler Logic:
    let mut scheduler_pairs: Vec<(String, Value)> = Vec::new();
    for row in rows.clone() {
        let mut r = row.clone();
        let id = r.as_object_mut()
            .and_then(|obj| obj.get("id").and_then(|v| v.as_str()).map(String::from)).unwrap();
        let raw_id = id.strip_prefix("game_database:").unwrap_or(&id).to_string();
        scheduler_pairs.push((raw_id, r));
    }
    let hash_scheduler = hash_table(scheduler_pairs);
    println!("Scheduler Hash: {}", hash_scheduler);

    // 2. SSP Logic:
    let mut ssp_pairs: Vec<(String, Value)> = Vec::new();
    for row in rows {
        let mut r = row.clone();
        let id = r.as_object_mut()
            .and_then(|obj| obj.get("id").and_then(|v| v.as_str()).map(String::from)).unwrap();
        let raw_id = id.strip_prefix("game_database:").unwrap_or(&id).to_string();

        // Simulate Sp00kyValue
        if let serde_json::Value::Object(map) = &mut r {
            for (_, v) in map.iter_mut() {
                if let serde_json::Value::Number(n) = v {
                    if let Some(i) = n.as_i64() {
                        *v = json!(i);
                    }
                }
            }
        }
        ssp_pairs.push((raw_id, r));
    }
    let hash_ssp = hash_table(ssp_pairs);
    println!("SSP Hash: {}", hash_ssp);
}
