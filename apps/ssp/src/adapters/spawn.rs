use ssp_node::{ports::LocalBoxFuture, Spawner};

/// `ssp_node::Spawner` on tokio — fire-and-forget tails.
pub struct TokioSpawner;

impl Spawner for TokioSpawner {
    fn spawn(&self, fut: LocalBoxFuture) {
        tokio::spawn(fut);
    }
}
