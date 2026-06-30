use serde_json::json;

fn main() {
    let json_str = r#"{"id": "6gioxblv1fnvq1o8rnmk", "locked": true, "name": "Chess.com", "owner": "user:eb0wzu5qq1a3z666tfi9", "source": "chesscom"}"#;
    let val: serde_json::Value = serde_json::from_str(json_str).unwrap();
    println!("val: {:?}", val);
    
    // Simulate Sp00kyValue conversion
    // ... wait, I don't have Sp00kyValue here.
}
