use surrealdb::engine::local::RocksDb;
use surrealdb::Surreal;

#[tokio::main]
async fn main() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Surreal::new::<RocksDb>(tmp.path()).await.unwrap();
    db.use_ns("test").use_db("test").await.unwrap();
    
    // Insert twice
    let records = vec![
        serde_json::json!({"id": "foo", "val": 1}),
        serde_json::json!({"id": "bar", "val": 2}),
        serde_json::json!({"id": "foo", "val": 1}),
        serde_json::json!({"id": "bar", "val": 2}),
    ];
    let mut resp = db.query("INSERT INTO game $records RETURN NONE")
        .bind(("records", records))
        .await.unwrap();
    resp.check().unwrap();

    let mut r = db.query("SELECT * FROM game").await.unwrap();
    let rows: Vec<serde_json::Value> = r.take(0).unwrap();
    println!("Rows: {}", rows.len());
}
