use anyhow::Context;
use axum::{
    extract::{State, Json},
    routing::post,
    Router,
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use spooky_stream_processor::{Circuit, StreamProcessor, MaterializedViewUpdate};
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;

#[derive(Clone)]
struct AppState {
    db: Surreal<Client>,
    processor: Arc<Mutex<Box<dyn StreamProcessor>>>,
}

#[derive(Deserialize)]
struct IngestRequest {
    table: String,
    op: String,
    id: String,
    record: Value,
    hash: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("Starting spooky-sidecar...");

    // Connect to SurrealDB
    // Using default local address, can be configured via env var if needed
    let db_addr = std::env::var("SURREALDB_ADDR").unwrap_or_else(|_| "127.0.0.1:8000".to_string());
    let db = Surreal::new::<Ws>(db_addr).await.context("Failed to connect to SurrealDB")?;
    
    // Auth - default root/root
    db.signin(Root {
        username: "root",
        password: "root",
    }).await.context("Failed to signin")?;

    db.use_ns("test").use_db("test").await.context("Failed to select ns/db")?;

    println!("Connected to SurrealDB");

    // Initialize Circuit
    let processor = Circuit::new();
    let processor_boxed: Box<dyn StreamProcessor> = Box::new(processor);

    let state = AppState {
        db,
        processor: Arc::new(Mutex::new(processor_boxed)),
    };

    let app = Router::new()
        .route("/ingest", post(ingest_handler))
        .with_state(state);

    let listener_addr = "0.0.0.0:3000";
    let listener = tokio::net::TcpListener::bind(listener_addr).await.context("Failed to bind port")?;
    println!("Listening on {}", listener_addr);
    
    axum::serve(listener, app).await.context("Server error")?;

    Ok(())
}

async fn ingest_handler(
    State(state): State<AppState>,
    Json(payload): Json<IngestRequest>,
) -> Json<Vec<MaterializedViewUpdate>> {
    // Process record
    let updates = {
        let mut processor = state.processor.lock().unwrap();
        processor.ingest_record(
            payload.table,
            payload.op,
            payload.id,
            payload.record,
            payload.hash
        )
    };

    // Update records in SurrealDB
    for update in &updates {
        println!("Updating view {} hash {}", update.query_id, update.result_hash);
        // Upsert the materialized view state
        let query_id = update.query_id.clone();
        let tree = update.tree.clone();

        let _: Result<Option<Value>, _> = state.db
            .update(("materialized_views", query_id))
            .content(tree)
            .await;
    }

    Json(updates)
}
