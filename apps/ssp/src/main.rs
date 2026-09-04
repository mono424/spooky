use ssp_server::run_server;

fn main() -> anyhow::Result<()> {
    // Explicit worker count instead of `#[tokio::main]`'s
    // `available_parallelism()`, for the same reason the scheduler does it
    // (see apps/scheduler/src/main.rs): production deploys default to a 1-vCPU
    // cgroup, and `available_parallelism()` is cgroup-aware, so the runtime
    // comes up with a SINGLE worker thread. One CPU-bound stretch then parks
    // the whole runtime — no IO polling, no timers, total HTTP silence — while
    // the process stays alive and `docker ps` still says "running".
    //
    // For the SSP that stretch is bootstrap: loading a large tenant's tables
    // into the circuit. Observed 2026-09-04 on a tenant with 380k rows in one
    // table, where the load ran past the control plane's 90s startup grace
    // (`infra_liveness.go`) with `/health` unanswerable the whole time. The
    // probe correctly concluded "not answering", the orchestrator recreated
    // the container, and the fresh one started the same too-long bootstrap:
    // a livelock that only stopped because auto-heal hit its 3/hour cap.
    //
    // `/health` costs one `RwLock::read`, so a single spare worker is enough
    // to keep it answering while another thread does the bulk load.
    let workers = std::env::var("SPKY_WORKER_THREADS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(4);
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()?
        .block_on(run_server())
}
