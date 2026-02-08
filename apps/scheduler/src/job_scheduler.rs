use anyhow::Result;

/// Job scheduler that watches for new jobs and dispatches them to SSPs
pub struct JobScheduler {
    // TODO: implement job scheduling logic
}

impl JobScheduler {
    pub fn new() -> Self {
        Self {}
    }

    /// Start watching for jobs
    pub async fn start(&self) -> Result<()> {
        // TODO: implement
        Ok(())
    }

    /// Shutdown the job scheduler
    pub async fn shutdown(&self) -> Result<()> {
        // TODO: implement
        Ok(())
    }
}
