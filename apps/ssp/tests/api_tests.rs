use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt; // for `oneshot`
use ssp_server::{create_app, load_config, connect_database, persistence, AppState, background_saver::BackgroundSaver, metrics};
use std::sync::Arc;
use tokio::sync::RwLock;

#[tokio::test]
async fn test_health_endpoint() {
    dotenvy::dotenv().ok();
    
    // We need a real or mocked DB. For integration tests, we'll try to use the dev configuration.
    // If DB is not available, this test will fail, which is acceptable for "Integration Tests".
    let config = load_config();
    let db = match connect_database(&config).await {
        Ok(db) => db,
        Err(_) => {
            println!("Skipping test: Could not connect to SurrealDB");
            return;
        }
    };

    let processor = persistence::load_circuit(&config.persistence_path);
    let processor_arc = Arc::new(RwLock::new(processor));
    
    let saver = Arc::new(BackgroundSaver::new(
        config.persistence_path.clone(),
        processor_arc.clone(),
        config.debounce_ms,
    ));

    // Initialize metrics (we can ignore the provider for tests)
    let (_, m) = metrics::init_metrics().unwrap();
    let metrics = Arc::new(m);

    let state = AppState {
        db,
        processor: processor_arc,
        persistence_path: config.persistence_path,
        saver,
        metrics,
    };

    let app = create_app(state);

    let response = app
        .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_version_endpoint() {
    // Version endpoint interacts with DB? No. 
    // But create_app requires state which requires DB.
    // We can't easily mock SharedDb (Arc<Surreal<Client>>) without a real connection or thorough mocking framework.
    // So we repeat the setup or make a helper.
    
    dotenvy::dotenv().ok();
    let config = load_config();
    let db = match connect_database(&config).await {
        Ok(db) => db,
        Err(_) => return, // Skip
    };
    
    let processor = persistence::load_circuit(&config.persistence_path);
    let processor_arc = Arc::new(RwLock::new(processor));
    let saver = Arc::new(BackgroundSaver::new(config.persistence_path.clone(), processor_arc.clone(), config.debounce_ms));
    let (_, m) = metrics::init_metrics().unwrap();
    
    let state = AppState {
        db,
        processor: processor_arc,
        persistence_path: config.persistence_path,
        saver,
        metrics: Arc::new(m),
    };
    
    let app = create_app(state);

    let response = app
        .oneshot(Request::builder().uri("/version").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}
