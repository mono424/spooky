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
            let parsed = serde_json::from_str::<Value>(&s).unwrap_or(Value::Null);
            println!("DEBUG: normalize_record parsed string record: {:?}", parsed);
            parsed
        },
        _ => {
            println!("DEBUG: normalize_record received direct record: {:?}", record);
            record
        }
    }
}

pub fn parse_params(params: Value) -> Option<Value> {
    let s = match params {
        Value::String(s) => fix_surql_json(&s),
        _ => params.to_string(),
    };
    serde_json::from_str(&s).ok()
}
