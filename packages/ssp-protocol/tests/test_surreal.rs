use ssp_protocol::snapshot_hash::hash_table;
use serde_json::json;

#[test]
fn test_spooky_hash() {
    let json_str1 = r#"{"count":221,"created_at":"2026-06-27T18:34:58.138066211Z","icon_type":"chesscom","id":"game_database:6gioxblv1fnvq1o8rnmk","locked":true,"name":"Chess.com","owner":"user:eb0wzu5qq1a3z666tfi9","share_urlname":"llDk_vd_bRGl","share_visibility":"public","source":"chesscom"}"#;
    let val1: serde_json::Value = serde_json::from_str(json_str1).unwrap();
    let json_str2 = r#"{"count":19,"created_at":"2026-06-27T18:34:55.759276239Z","icon_type":"database","id":"game_database:DBS_5c3r96v","locked":false,"name":"Mobile","owner":"user:eb0wzu5qq1a3z666tfi9","source":"mobile"}"#;
    let val2: serde_json::Value = serde_json::from_str(json_str2).unwrap();
    let json_str3 = r#"{"count":1410,"created_at":"2026-06-27T13:37:32.933765436Z","icon_type":"database","id":"game_database:DBS_5svdkGk","locked":false,"name":"Tal","owner":"user:eb0wzu5qq1a3z666tfi9"}"#;
    let val3: serde_json::Value = serde_json::from_str(json_str3).unwrap();
    let json_str4 = r#"{"count":3721,"created_at":"2026-06-27T18:33:52.084997145Z","icon_type":"lichess","id":"game_database:glwydf2hkbeuyf6veofd","locked":true,"name":"Lichess","owner":"user:eb0wzu5qq1a3z666tfi9","share_urlname":"3tfddZPE4op7","share_visibility":"private","source":"lichess"}"#;
    let val4: serde_json::Value = serde_json::from_str(json_str4).unwrap();

    let mut pairs = vec![
        ("6gioxblv1fnvq1o8rnmk".to_string(), val1.clone()),
        ("DBS_5c3r96v".to_string(), val2.clone()),
        ("DBS_5svdkGk".to_string(), val3.clone()),
        ("glwydf2hkbeuyf6veofd".to_string(), val4.clone()),
    ];

    let hash_direct = hash_table(pairs.clone());
    println!("Direct Hash: {}", hash_direct);

    for pair in pairs.iter_mut() {
        if let serde_json::Value::Object(map) = &mut pair.1 {
            for (_, v) in map.iter_mut() {
                if let serde_json::Value::Number(n) = v {
                    if let Some(i) = n.as_i64() {
                        *v = json!(i);
                    }
                }
            }
        }
    }

    let hash_sp00ky = hash_table(pairs.clone());
    println!("Sp00ky Hash: {}", hash_sp00ky);
    assert_eq!(hash_direct, hash_sp00ky);
}
