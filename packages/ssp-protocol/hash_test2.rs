use serde_json::Value;
use std::collections::BTreeMap;

fn canonical_json(value: &Value) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    write_canonical(value, &mut buf);
    buf
}

fn write_canonical(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push(b'{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 { out.push(b','); }
                let escaped = serde_json::to_vec(&Value::String((*k).clone())).unwrap();
                out.extend_from_slice(&escaped);
                out.push(b':');
                write_canonical(&map[*k], out);
            }
            out.push(b'}');
        }
        Value::Array(arr) => {
            out.push(b'[');
            for (i, v) in arr.iter().enumerate() {
                if i > 0 { out.push(b','); }
                write_canonical(v, out);
            }
            out.push(b']');
        }
        other => {
            let bytes = serde_json::to_vec(other).unwrap();
            out.extend_from_slice(&bytes);
        }
    }
}

fn hash_table(mut pairs: Vec<(String, Value)>) -> String {
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = blake3::Hasher::new();
    for (id, value) in &pairs {
        hasher.update(id.as_bytes());
        hasher.update(b"\0");
        let canonical = canonical_json(value);
        hasher.update(&canonical);
        hasher.update(b"\0");
    }
    format!("b3:{}", hasher.finalize().to_hex())
}

fn strip_reserved_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(k, v)| !k.starts_with("_00_") && !v.is_null())
                .collect(),
        ),
        other => other,
    }
}

fn main() {
    let json_str = r#"[{"_00_rv":134,"color":"#81B64C","count":221,"created_at":"2026-06-27T18:34:58.138066211Z","icon_type":"chesscom","id":"game_database:6gioxblv1fnvq1o8rnmk","locked":true,"name":"Chess.com","owner":"user:eb0wzu5qq1a3z666tfi9","player_name":null,"share_urlname":"llDk_vd_bRGl","share_visibility":"public","source":"chesscom"},{"_00_rv":106,"color":"#56CCF2","count":19,"created_at":"2026-06-27T18:34:55.759276239Z","icon_type":"database","id":"game_database:DBS_5c3r96v","locked":false,"name":"Mobile","owner":"user:eb0wzu5qq1a3z666tfi9","player_name":null,"share_urlname":null,"share_visibility":null,"source":"mobile"},{"_00_rv":63,"color":"#F2C94C","count":1410,"created_at":"2026-06-27T13:37:32.933765436Z","icon_type":"database","id":"game_database:DBS_5svdkGk","locked":false,"name":"Tal","owner":"user:eb0wzu5qq1a3z666tfi9","player_name":null,"share_urlname":null,"share_visibility":null,"source":null},{"_00_rv":104,"color":"#FFFFFF","count":3721,"created_at":"2026-06-27T18:33:52.084997145Z","icon_type":"lichess","id":"game_database:glwydf2hkbeuyf6veofd","locked":true,"name":"Lichess","owner":"user:eb0wzu5qq1a3z666tfi9","player_name":null,"share_urlname":"3tfddZPE4op7","share_visibility":"private","source":"lichess"}]"#;
    let rows: Vec<Value> = serde_json::from_str(json_str).unwrap();
    
    // WITHOUT ID
    let mut table_rows = Vec::new();
    for mut row in rows.clone() {
        let id_str = row.get("id").unwrap().as_str().unwrap().to_string();
        let raw_id = id_str.strip_prefix("game_database:").unwrap_or(&id_str).to_string();
        
        if let Value::Object(map) = &mut row {
            map.remove("id");
        }
        table_rows.push((raw_id, strip_reserved_keys(row)));
    }
    println!("HASH WITHOUT ID: {}", hash_table(table_rows));
    
    // WITH ID
    let mut table_rows2 = Vec::new();
    for row in rows {
        let id_str = row.get("id").unwrap().as_str().unwrap().to_string();
        let raw_id = id_str.strip_prefix("game_database:").unwrap_or(&id_str).to_string();
        
        table_rows2.push((raw_id, strip_reserved_keys(row)));
    }
    println!("HASH WITH ID: {}", hash_table(table_rows2));
}
