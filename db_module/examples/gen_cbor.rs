use serde_json::json;

fn main() {
    let complex_data = json!({
        "id": "user:abc123",
        "name": "Alice",
        "age": 28,
        "score": 99.5,
        "active": true,
        "deleted": false,
        "metadata": null,
        "tags": ["developer", "rust", "database"],
        "count": 1000u64,
        "profile": {
            "bio": "Software engineer",
            "avatar": "https://example.com/avatar.jpg",
            "settings": {
                "theme": "dark",
                "notifications": true,
                "privacy": {
                    "public": false,
                    "level": 3
                }
            }
        },
        "history": [
            {"action": "login", "timestamp": 1234567890},
            {"action": "update", "timestamp": 1234567900}
        ],
        "mixed_array": [42, "text", true, {"nested": "value"}]
    });
    
    let cbor_val: cbor4ii::core::Value = serde_json::from_value(complex_data).unwrap();
    let buf = cbor4ii::serde::to_vec(Vec::new(), &cbor_val).unwrap();
    
    // Print as Rust byte array
    print!("const BENCH_CBOR: &[u8] = &[\n    ");
    for (i, byte) in buf.iter().enumerate() {
        if i > 0 && i % 16 == 0 {
            print!("\n    ");
        }
        print!("{}, ", byte);
    }
    println!("\n];");
}
