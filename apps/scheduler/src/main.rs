use anyhow::Result;
use scheduler::{Scheduler, config::SchedulerConfig, transport::NatsTransport};
use std::sync::Arc;
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        )
        .init();

    // Load configuration
    let config = SchedulerConfig::load()?;

    // Create NATS transport
    let transport = Arc::new(NatsTransport::new(&config.nats).await?);

    // Create and start scheduler
    let scheduler = Scheduler::new(config, transport);
    
    // Handle graceful shutdown
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
    
    tokio::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to listen for ctrl-c");
        let _ = shutdown_tx.send(());
    });

    // Start the scheduler
    tokio::select! {
        result = scheduler.start() => {
            if let Err(e) = result {
                eprintln!("Scheduler error: {}", e);
                std::process::exit(1);
            }
        }
        _ = &mut shutdown_rx => {
            scheduler.shutdown().await?;
        }
    }

    Ok(())
}
