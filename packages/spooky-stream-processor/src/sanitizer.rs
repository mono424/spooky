use regex::{Regex, Captures};
use serde_json::Value;

// --- Helper: Fix SurrealQL object string to valid JSON ---
pub fn fix_surql_json(s: &str) -> String {
    println!("DEBUG: fix_surql_json input: {}", s);
    
    // Regex to match:
    // 1. Single quoted strings: '...' (Group 1)
    // 2. Double quoted strings: "..." (Group 2)
    // 3. Keys: identifier followed by colon AND space (e.g. "id: "). (Group 3)
    // 4. Record IDs (Simple): ident:ident (e.g. user:123). (Group 4)
    // 5. Record IDs (Backticked): ident:`...` (e.g. thread:`thread:123`). (Group 5)
    let re = Regex::new(r#"('[^']*')|("[^"]*")|(\w+)\s*:\s|(\w+:\w+)|(\w+:`[^`]+`)"#).unwrap();
    
    let result = re.replace_all(s, |caps: &Captures| {
        if let Some(key) = caps.get(3) {
             // Key found. Quote it.
            format!("\"{}\": ", key.as_str())
        } else if let Some(rec_id) = caps.get(4) {
            // Unquoted Record ID (simple). Quote it.
             format!("\"{}\"", rec_id.as_str())
        } else if let Some(rec_id_complex) = caps.get(5) {
             // Complex Record ID with backticks.
             // Strategy: Extract the content inside backticks.
             let raw = rec_id_complex.as_str();
             if let Some(start) = raw.find('`') {
                 if let Some(end) = raw.rfind('`') {
                     let inner = &raw[start+1..end];
                     return format!("\"{}\"", inner);
                 }
             }
             format!("\"{}\"", raw)
        } else if let Some(sq) = caps.get(1) {
            let content = &sq.as_str()[1..sq.as_str().len()-1];
            let escaped = content.replace("\"", "\\\"");
            format!("\"{}\"", escaped)
        } else {
             caps.get(0).unwrap().as_str().to_string()
        }
    });

    println!("DEBUG: fix_surql_json output: {}", result);
    result.to_string()
}

pub fn normalize_record(record: Value) -> Value {
    match record {
        Value::String(s) => {
             // Try to parse string as JSON, if it looks like it
             // But be careful not to double parse simple strings
             if (s.starts_with('{') && s.ends_with('}')) || (s.starts_with('[') && s.ends_with(']')) {
                 if let Ok(parsed) = serde_json::from_str::<Value>(&s) {
                     // Recurse on the parsed value
                     return normalize_record(parsed);
                 }
             }
             // Otherwise return the string as is (SurrealDB string IDs are just strings)
             Value::String(s)
        },
        Value::Object(map) => {
            // Check for SurrealDB Record ID object format { tb: "table", id: "id" }
            if map.len() == 2 && map.contains_key("tb") && map.contains_key("id") {
                let tb = map.get("tb").and_then(|v| v.as_str());
                let id = map.get("id");
                
                if let (Some(tb_str), Some(id_val)) = (tb, id) {
                    let id_str = match id_val {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                         _ => id_val.to_string() 
                    };
                    
                    // Format: table:id
                    // Note: If id already contains table prefix (which happens sometimes), don't double it?
                    // But standard { tb: "t", id: "i" } usually means t:i
                    return Value::String(format!("{}:{}", tb_str, id_str));
                }
            }
            
            // Otherwise recurse on all fields
            let mut new_map = serde_json::Map::new();
            for (k, v) in map {
                new_map.insert(k, normalize_record(v));
            }
            Value::Object(new_map)
        },
        Value::Array(arr) => {
            Value::Array(arr.into_iter().map(normalize_record).collect())
        },
        _ => record
    }
}

pub fn parse_params(params: Value) -> Option<Value> {
    let s = match params {
        Value::String(s) => fix_surql_json(&s),
        _ => params.to_string(),
    };
    serde_json::from_str(&s).ok()
}
