use anyhow::Result;
use scheduler::config::SchedulerConfig;
use scheduler::transport::HttpTransport;
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    info!("\n ____        _              _       _\n/ ___|  ___| |__   ___  __| |_   _| | ___ _ __\n\\___ \\ / __| '_ \\ / _ \\/ _` | | | | |/ _ \\ '__|\n ___) | (__| | | |  __/ (_| | |_| | |  __/ |\n|____/ \\___|_| |_|\\___\\|\\__,_|\\__,_|_|\\___|_|    v{}\n\nSp00ky Cluster Scheduler", env!("CARGO_PKG_VERSION"));

    // Load configuration
    let config = SchedulerConfig::load()?;
    
    // Initialize transport (HTTP)
    let transport = Arc::new(HttpTransport::new());
    
    // Create scheduler
    let scheduler = scheduler::Scheduler::new(config.clone(), transport.clone()).await?;
    
    // Create shared trackers for state consistency
    let query_tracker = std::sync::Arc::new(scheduler::query::QueryTracker::new());
    let job_tracker = std::sync::Arc::new(scheduler::job_scheduler::JobTracker::new());
    
    // Create HTTP server with all routers
    let ingest_router = scheduler::ingest::create_ingest_router(scheduler.ingest_state());
    
    let query_state = scheduler::query::QueryState {
        ssp_pool: std::sync::Arc::clone(&scheduler.ssp_pool),
        transport: std::sync::Arc::clone(&transport),
        query_tracker: std::sync::Arc::clone(&query_tracker),
    };
    let query_router = scheduler::query::create_query_router(query_state.clone());
    
    let job_state = scheduler::job_scheduler::JobState {
        ssp_pool: std::sync::Arc::clone(&query_state.ssp_pool),
        transport: std::sync::Arc::clone(&transport),
        job_tracker: std::sync::Arc::clone(&job_tracker),
    };
    let job_router = scheduler::job_scheduler::create_job_router(job_state.clone());

    let ssp_mgmt_state = scheduler::ssp_management::SspManagementState {
        ssp_pool: std::sync::Arc::clone(&query_state.ssp_pool),
        replica: scheduler.replica.clone(),
        transport: std::sync::Arc::clone(&transport),
        config: std::sync::Arc::new(config.clone()),
        status: scheduler.status.clone(),
        event_buffer: scheduler.event_buffer.clone(),
    };
    let ssp_router = scheduler::ssp_management::create_ssp_router(ssp_mgmt_state);

    let proxy_router = scheduler::proxy::create_proxy_router(scheduler.proxy_state());

    // Create backend health cache and shared configs for live updates
    let backend_health_cache = scheduler::backend_health::create_health_cache(&config.backends);
    let shared_backend_configs = scheduler::backend_health::create_shared_configs(&config.backends);
    scheduler::backend_health::start_backend_health_monitor(
        shared_backend_configs.clone(),
        backend_health_cache.clone(),
        config.health_check_interval_secs,
    );

    let metrics_router = scheduler::metrics::create_metrics_router(
        scheduler.metrics_state(
            std::sync::Arc::clone(&query_tracker),
            std::sync::Arc::clone(&job_tracker),
            backend_health_cache,
            shared_backend_configs,
        )
    );
    
    let backup_config = Arc::new(scheduler::backup::BackupConfig::from_env());
    let backup_registry = Arc::new(scheduler::backup::BackupRegistry::new());
    let (backup_tx, backup_rx) = scheduler::backup::create_backup_channel();
    let backup_router = scheduler::backup::create_backup_router(scheduler.backup_state(
        Arc::clone(&backup_registry),
        backup_tx.clone(),
        Arc::clone(&backup_config),
    ));

    let app = axum::Router::new()
        .merge(ingest_router)
        .merge(query_router)
        .merge(job_router)
        .merge(ssp_router)
        .merge(proxy_router)
        .merge(metrics_router)
        .merge(backup_router);
    
    let ingest_addr = format!("{}:{}", 
        scheduler.config().ingest_host.as_deref().unwrap_or("0.0.0.0"),
        scheduler.config().ingest_port
    );
    
    // Start background monitors
    scheduler::metrics::start_query_reassignment_monitor(
        std::sync::Arc::clone(&query_state.ssp_pool),
        std::sync::Arc::clone(&query_tracker),
    ).await;
    
    scheduler::job_scheduler::start_job_failover_monitor(
        std::sync::Arc::clone(&job_state.ssp_pool),
        std::sync::Arc::clone(&job_tracker),
        std::sync::Arc::clone(&transport),
    ).await;

    // Spawn the single-consumer backup worker
    {
        let replica = scheduler.replica.clone();
        let ingest = scheduler.ingest_state();
        let config = Arc::clone(&backup_config);
        let registry = Arc::clone(&backup_registry);
        tokio::spawn(async move {
            scheduler::backup::run_backup_worker(backup_rx, replica, ingest, config, registry).await;
        });
    }

    info!("Started background monitors for query reassignment, job failover, and backups");
    
    // Spawn HTTP server
    let server_handle = tokio::spawn(async move {
        info!("Starting HTTP server on {}...", ingest_addr);
        let listener = tokio::net::TcpListener::bind(&ingest_addr)
            .await
            .expect("Failed to bind port");
        
        axum::serve(listener, app)
            .await
            .expect("HTTP server failed");
    });
    
    // Handle graceful shutdown
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
    
    tokio::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to listen for ctrl-c");
        info!("Received shutdown signal");
        let _ = shutdown_tx.send(());
    });
    
    // Start scheduler
    let scheduler_handle = tokio::spawn(async move {
        if let Err(e) = scheduler.start().await {
            eprintln!("Scheduler error: {}", e);
        }
    });
    
    // Wait for shutdown or error
    tokio::select! {
        _ = &mut shutdown_rx => {
            info!("Shutting down...");
        }
        _ = server_handle => info!("HTTP server stopped"),
        _ = scheduler_handle => info!("Scheduler stopped"),
    }

    Ok(())
}
