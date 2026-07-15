use super::MaybeSendSync;

#[cfg(not(target_arch = "wasm32"))]
pub type LocalBoxFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>;
#[cfg(target_arch = "wasm32")]
pub type LocalBoxFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'static>>;

/// Fire-and-forget tails ONLY: post-ingest fan-out, list_ref delete cleanups,
/// starting the job-consumer task at init. Never for periodic or durable work
/// — that's [`super::Scheduler`]. Work spawned here may be lost on process
/// restart / DO eviction by design (the recovery sweep is the backstop).
///
/// VM adapter: `tokio::spawn`. CF adapter: `spawn_local` / `ctx.wait_until`.
pub trait Spawner: MaybeSendSync {
    fn spawn(&self, fut: LocalBoxFuture);
}
