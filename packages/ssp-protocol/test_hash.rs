use serde_json::Value;

fn main() {
    let json_str = r#"[{"_00_rv":134,"color":"#81B64C","count":221,"created_at":"2026-06-27T18:34:58.138066211Z","icon_type":"chesscom","id":"game_database:6gioxblv1fnvq1o8rnmk","locked":true,"name":"Chess.com","owner":"user:eb0wzu5qq1a3z666tfi9","player_name":null,"share_urlname":"llDk_vd_bRGl","share_visibility":"public","source":"chesscom"},{"_00_rv":106,"color":"#56CCF2","count":19,"created_at":"2026-06-27T18:34:55.759276239Z","icon_type":"database","id":"game_database:DBS_5c3r96v","locked":false,"name":"Mobile","owner":"user:eb0wzu5qq1a3z666tfi9","player_name":null,"share_urlname":null,"share_visibility":null,"source":"mobile"},{"_00_rv":63,"color":"#F2C94C","count":1410,"created_at":"2026-06-27T13:37:32.933765436Z","icon_type":"database","id":"game_database:DBS_5svdkGk","locked":false,"name":"Tal","owner":"user:eb0wzu5qq1a3z666tfi9","player_name":null,"share_urlname":null,"share_visibility":null,"source":null},{"_00_rv":104,"color":"#FFFFFF","count":3721,"created_at":"2026-06-27T18:33:52.084997145Z","icon_type":"lichess","id":"game_database:glwydf2hkbeuyf6veofd","locked":true,"name":"Lichess","owner":"user:eb0wzu5qq1a3z666tfi9","player_name":null,"share_urlname":"3tfddZPE4op7","share_visibility":"private","source":"lichess"}]"#;
    let rows: Vec<Value> = serde_json::from_str(json_str).unwrap();
    let mut table_rows = Vec::new();
    for row in rows {
        let id_str = row.get("id").unwrap().as_str().unwrap().to_string();
        let raw_id = id_str.strip_prefix("game_database:").unwrap_or(&id_str).to_string();
        table_rows.push((raw_id, row));
    }
    let hash = ssp_protocol::snapshot_hash::hash_table(table_rows);
    println!("HASH: {}", hash);
}
