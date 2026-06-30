fn bootstrap_page_query(table: &str, limit: usize, after_id: Option<&str>) -> String {
    if let Some(id) = after_id {
        let raw = id.strip_prefix(&format!("{}:", table)).unwrap_or(id);
        format!("SELECT * FROM {} WHERE id > type::record('{}', '{}') ORDER BY id LIMIT {}", table, table, raw, limit)
    } else {
        format!("SELECT * FROM {} ORDER BY id LIMIT {}", table, limit)
    }
}
fn main() {
    println!("{}", bootstrap_page_query("game_database", 200, Some("game_database:abc-123")));
}
