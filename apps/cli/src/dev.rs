use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::backend::{SpookyConfig, ResolvedVersions};
use crate::migrate;
use crate::surreal_client::{MigrationDB, SurrealClient};

const PREFIX: &str = "[spooky dev]";

const NETWORK_NAME: &str = "spooky-dev-net";
const SURREAL_CONTAINER: &str = "spooky-dev-surrealdb";
const SSP_CONTAINER: &str = "spooky-dev-ssp";
const SCHEDULER_CONTAINER: &str = "spooky-dev-scheduler";

const SURREAL_PORT: u16 = 8666;
const SSP_PORT: u16 = 8667;
const SCHEDULER_PORT: u16 = 9667;
const HEALTH_MAX_RETRIES: u32 = 30;
const HEALTH_RETRY_INTERVAL: Duration = Duration::from_secs(2);

const INFRA_SERVICES_SINGLENODE: &[&str] = &["surrealdb", "aspire-dashboard"];
const INFRA_SERVICES_CLUSTER: &[&str] = &["surrealdb"];
const INFRA_SERVICES_SURREALISM: &[&str] = &["surrealdb"];

// ── Public entry point ──────────────────────────────────────────────────────

pub fn run() -> Result<()> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();
    ctrlc::set_handler(move || {
        stop_clone.store(true, Ordering::SeqCst);
    })
    .context("Failed to set Ctrl+C handler")?;

    println!("{} Starting development environment...", PREFIX);

    // Read config from spooky.yml
    let config = read_config("spooky.yml");
    let mode = config.mode.clone().unwrap_or_else(|| "singlenode".to_string());
    let versions = ResolvedVersions::from_config(&config);
    println!("{} Mode: {}", PREFIX, mode);

    // Check for compose files
    let compose_file = format!("docker-compose.{}.yml", mode);
    if Path::new(&compose_file).exists() {
        println!("{} Found compose file: {}", PREFIX, compose_file);
        run_compose_mode(&compose_file, &mode, &stop)
    } else {
        println!("{} No compose file found — using direct Docker mode", PREFIX);
        run_direct_mode(&mode, &versions, &stop)
    }
}

// ── Config reading ──────────────────────────────────────────────────────────

fn default_config() -> SpookyConfig {
    SpookyConfig {
        mode: Some("singlenode".to_string()),
        surrealdb: None,
        version: None,
        backends: Default::default(),
        buckets: Default::default(),
        client_types: Default::default(),
    }
}

fn read_config(config_path: &str) -> SpookyConfig {
    let path = Path::new(config_path);
    if !path.exists() {
        return default_config();
    }

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return default_config(),
    };

    match serde_yaml::from_str(&content) {
        Ok(c) => c,
        Err(_) => default_config(),
    }
}

// ── Direct Docker mode ──────────────────────────────────────────────────────

fn run_direct_mode(mode: &str, versions: &ResolvedVersions, stop: &Arc<AtomicBool>) -> Result<()> {
    let surreal_image = versions.surrealdb_image();
    let ssp_image = versions.ssp_image();

    // Clean up any stale resources from a previous run
    let _ = docker(&["rm", "-f", SURREAL_CONTAINER]);
    let _ = docker(&["rm", "-f", SSP_CONTAINER]);
    let _ = docker(&["rm", "-f", SCHEDULER_CONTAINER]);
    let _ = docker(&["network", "rm", NETWORK_NAME]);

    // Phase 1: Create Docker network
    println!("\n{} Phase 1: Creating Docker network...", PREFIX);
    docker(&["network", "create", NETWORK_NAME])?;

    // Phase 2: Start SurrealDB
    println!("{} Phase 2: Starting SurrealDB...", PREFIX);
    docker(&[
        "run", "-d",
        "--name", SURREAL_CONTAINER,
        "--network", NETWORK_NAME,
        "--network-alias", "surrealdb",
        "-p", &format!("{}:8000", SURREAL_PORT),
        "-e", "SURREAL_USER=root",
        "-e", "SURREAL_PASS=root",
        "-e", "SURREAL_LOG=info",
        &surreal_image,
        "start",
        "--bind", "0.0.0.0:8000",
        "--allow-all",
        "--user", "root",
        "--pass", "root",
        "memory",
    ])?;

    // Phase 3: Wait for health
    println!("{} Phase 3: Waiting for SurrealDB health...", PREFIX);
    wait_for_health(
        &format!("http://localhost:{}/health", SURREAL_PORT),
        HEALTH_MAX_RETRIES,
        HEALTH_RETRY_INTERVAL,
        stop,
    )?;

    if stop.load(Ordering::SeqCst) {
        return cleanup_direct(stop);
    }

    // Phase 4: Apply migrations
    println!("{} Phase 4: Applying migrations...", PREFIX);
    apply_migrations(SURREAL_PORT)?;

    if stop.load(Ordering::SeqCst) {
        return cleanup_direct(stop);
    }

    // Phase 4b: Patch endpoint functions for Docker networking
    println!("{} Phase 4b: Patching endpoint functions...", PREFIX);
    patch_endpoints(SURREAL_PORT, mode)?;

    if stop.load(Ordering::SeqCst) {
        return cleanup_direct(stop);
    }

    // Phase 5: Start SSP
    println!("{} Phase 5: Starting SSP...", PREFIX);
    let config_mount = std::env::current_dir()
        .context("Failed to get current directory")?
        .join("spooky.yml");
    let data_dir = std::env::current_dir()
        .context("Failed to get current directory")?
        .join(".spooky/ssp_data");

    // Ensure data dir exists
    std::fs::create_dir_all(&data_dir).ok();

    let port_mapping = format!("{}:8667", SSP_PORT);
    let config_mount_str = format!("{}:/config/spooky.yml:ro", config_mount.display());
    let data_mount_str = format!("{}:/data", data_dir.display());

    let scheduler_url_env = format!("SCHEDULER_URL=http://scheduler:{}", SCHEDULER_PORT);
    let advertise_addr_env = format!("ADVERTISE_ADDR={}:{}", SSP_CONTAINER, SSP_PORT);

    let mut ssp_args = vec![
        "run", "-d",
        "--platform", "linux/amd64",
        "--name", SSP_CONTAINER,
        "--network", NETWORK_NAME,
        "--network-alias", "ssp",
        "-p", &port_mapping,
        "-e", "RUST_LOG=info,ssp=debug",
        "-e", "SURREALDB_ADDR=surrealdb:8000/rpc",
        "-e", "SURREALDB_NS=main",
        "-e", "SURREALDB_DB=main",
        "-e", "SURREALDB_USER=root",
        "-e", "SURREALDB_PASS=root",
        "-e", "SPOOKY_AUTH_SECRET=mysecret",
        "-e", "SPOOKY_PERSISTENCE_FILE=/data/spooky_state.json",
        "-e", "SPOOKY_CONFIG_PATH=/config/spooky.yml",
    ];

    if mode == "cluster" {
        ssp_args.extend(["-e", &scheduler_url_env]);
        ssp_args.extend(["-e", "SSP_ID=ssp-1"]);
        ssp_args.extend(["-e", &advertise_addr_env]);
    }

    if config_mount.exists() {
        ssp_args.extend(["-v", &config_mount_str]);
    }
    ssp_args.extend(["-v", &data_mount_str]);
    ssp_args.push(&ssp_image);

    docker(&ssp_args)?;

    // Phase 6 (cluster only): Start scheduler
    let scheduler_log;
    if mode == "cluster" {
        let scheduler_image = versions.scheduler_image();
        let scheduler_port_mapping = format!("{}:9667", SCHEDULER_PORT);

        println!("{} Phase 6: Starting scheduler...", PREFIX);
        docker(&[
            "run", "-d",
            "--platform", "linux/amd64",
            "--name", SCHEDULER_CONTAINER,
            "--network", NETWORK_NAME,
            "--network-alias", "scheduler",
            "-p", &scheduler_port_mapping,
            "-e", "RUST_LOG=info",
            "-e", "SPOOKY_SCHEDULER_DB_URL=surrealdb:8000/rpc",
            "-e", "SPOOKY_SCHEDULER_DB_NAMESPACE=main",
            "-e", "SPOOKY_SCHEDULER_DB_DATABASE=main",
            "-e", "SPOOKY_SCHEDULER_DB_USERNAME=root",
            "-e", "SPOOKY_SCHEDULER_DB_PASSWORD=root",
            "-e", "SPOOKY_SCHEDULER_REPLICA_DB_PATH=/data/replica",
            "-e", "SPOOKY_SCHEDULER_WAL_PATH=/data/event_wal.log",
            "-e", "SPOOKY_AUTH_SECRET=mysecret",
            &scheduler_image,
        ])?;

        println!("{} Waiting for scheduler health...", PREFIX);
        wait_for_health(
            &format!("http://localhost:{}/metrics", SCHEDULER_PORT),
            HEALTH_MAX_RETRIES,
            HEALTH_RETRY_INTERVAL,
            stop,
        )?;

        scheduler_log = Some(spawn_log_tail(SCHEDULER_CONTAINER, "scheduler"));
    } else {
        scheduler_log = None;
    }

    // Ready!
    println!("\n{} Development environment ready!", PREFIX);
    println!("{} SurrealDB:  http://localhost:{}", PREFIX, SURREAL_PORT);
    println!("{} SSP:        http://localhost:{}", PREFIX, SSP_PORT);
    if mode == "cluster" {
        println!("{} Scheduler:  http://localhost:{}", PREFIX, SCHEDULER_PORT);
    }
    println!("{} Press Ctrl+C to stop.\n", PREFIX);

    // Tail logs from containers
    let surreal_log = spawn_log_tail(SURREAL_CONTAINER, "surrealdb");
    let ssp_log = spawn_log_tail(SSP_CONTAINER, "ssp");

    // Wait for Ctrl+C
    while !stop.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(250));
    }

    // Stop log tailers
    drop(surreal_log);
    drop(ssp_log);
    drop(scheduler_log);

    cleanup_direct(stop)
}

fn cleanup_direct(_stop: &Arc<AtomicBool>) -> Result<()> {
    println!("\n{} Shutting down...", PREFIX);

    // Remove containers (ignore errors — they might not exist)
    let _ = docker(&["rm", "-f", SCHEDULER_CONTAINER]);
    let _ = docker(&["rm", "-f", SSP_CONTAINER]);
    let _ = docker(&["rm", "-f", SURREAL_CONTAINER]);

    // Remove network
    let _ = docker(&["network", "rm", NETWORK_NAME]);

    println!("{} Cleaned up. Goodbye!", PREFIX);
    Ok(())
}

// ── Compose mode ────────────────────────────────────────────────────────────

fn run_compose_mode(compose_file: &str, mode: &str, stop: &Arc<AtomicBool>) -> Result<()> {
    let infra_services: &[&str] = match mode {
        "cluster" => INFRA_SERVICES_CLUSTER,
        "surrealism" => INFRA_SERVICES_SURREALISM,
        _ => INFRA_SERVICES_SINGLENODE,
    };

    // Phase 1: Start infrastructure
    println!(
        "\n{} Phase 1: Starting infrastructure ({})...",
        PREFIX,
        infra_services.join(", ")
    );
    let mut args = vec![
        "compose", "-f", compose_file, "up", "-d", "--remove-orphans",
    ];
    for svc in infra_services {
        args.push(svc);
    }
    docker(&args)?;

    if stop.load(Ordering::SeqCst) {
        return cleanup_compose(compose_file);
    }

    // Phase 2: Wait for SurrealDB health
    println!("\n{} Phase 2: Waiting for SurrealDB health...", PREFIX);
    wait_for_health(
        &format!("http://localhost:{}/health", SURREAL_PORT),
        HEALTH_MAX_RETRIES,
        HEALTH_RETRY_INTERVAL,
        stop,
    )?;

    if stop.load(Ordering::SeqCst) {
        return cleanup_compose(compose_file);
    }

    // Phase 3: Apply migrations
    println!("\n{} Phase 3: Applying migrations...", PREFIX);
    apply_migrations(SURREAL_PORT)?;

    if stop.load(Ordering::SeqCst) {
        return cleanup_compose(compose_file);
    }

    // Phase 4: Start remaining services (foreground)
    println!("\n{} Phase 4: Starting remaining services...", PREFIX);
    println!("{} Press Ctrl+C to stop.\n", PREFIX);

    let status = Command::new("docker")
        .args(["compose", "-f", compose_file, "up", "--remove-orphans"])
        .status()
        .context("Failed to run docker compose up")?;

    if !status.success() && !stop.load(Ordering::SeqCst) {
        bail!("docker compose up exited with status {}", status);
    }

    cleanup_compose(compose_file)
}

fn cleanup_compose(compose_file: &str) -> Result<()> {
    println!("\n{} Stopping compose services...", PREFIX);
    let _ = docker(&["compose", "-f", compose_file, "down", "--remove-orphans"]);
    println!("{} Cleaned up. Goodbye!", PREFIX);
    Ok(())
}

// ── Health checking ─────────────────────────────────────────────────────────

fn wait_for_health(
    url: &str,
    max_retries: u32,
    interval: Duration,
    stop: &Arc<AtomicBool>,
) -> Result<()> {
    for attempt in 1..=max_retries {
        if stop.load(Ordering::SeqCst) {
            bail!("Interrupted while waiting for SurrealDB");
        }

        match ureq::get(url).timeout(Duration::from_secs(5)).call() {
            Ok(resp) if resp.status() == 200 => {
                println!("{} SurrealDB is ready.", PREFIX);
                return Ok(());
            }
            _ => {
                println!(
                    "{} Waiting for SurrealDB... ({}/{})",
                    PREFIX, attempt, max_retries
                );
                thread::sleep(interval);
            }
        }
    }

    bail!(
        "SurrealDB did not become ready after {} attempts",
        max_retries
    );
}

// ── Migration helper ────────────────────────────────────────────────────────

fn apply_migrations(port: u16) -> Result<()> {
    let migrations_dir = Path::new("migrations");
    if !migrations_dir.exists() {
        println!("{} No migrations/ directory found, skipping.", PREFIX);
        return Ok(());
    }

    let client = SurrealClient::new(
        &format!("http://localhost:{}", port),
        "main",
        "main",
        "root",
        "root",
    );

    // Check for pending migrations and prompt user before applying
    client.ensure_ns_db().context("Failed to ensure namespace/database exist")?;
    client.ping().context("Cannot connect to SurrealDB")?;
    client.ensure_migration_table()?;

    let filesystem = migrate::scan_migrations(migrations_dir)?;
    let applied = client.get_applied_migrations()?;
    let applied_versions: Vec<&str> = applied.iter().map(|a| a.version.as_str()).collect();
    let pending: Vec<_> = filesystem
        .iter()
        .filter(|f| !applied_versions.contains(&f.version.as_str()))
        .collect();

    if pending.is_empty() {
        println!("{} No pending migrations.", PREFIX);
        return Ok(());
    }

    println!("{} Found {} pending migration(s):", PREFIX, pending.len());
    for m in &pending {
        println!("  - {}_{}", m.version, m.name);
    }

    let confirm = inquire::Confirm::new(&format!(
        "Apply {} pending migration(s)?",
        pending.len()
    ))
    .with_default(true)
    .prompt()
    .unwrap_or(false);

    if !confirm {
        bail!(
            "Cannot continue without applying migrations. \
             Run 'spooky migrate apply' manually or re-run 'spooky dev'."
        );
    }

    match migrate::apply(&client, migrations_dir) {
        Ok(()) => Ok(()),
        Err(e) => {
            println!("{} Migration failed: {}", PREFIX, e);
            let confirm = inquire::Confirm::new("Reset database and retry migrations?")
                .with_default(true)
                .prompt()
                .unwrap_or(false);
            if !confirm {
                bail!("Migration failed: {}", e);
            }
            println!("{} Resetting database and retrying...", PREFIX);
            client.reset_database()?;
            migrate::apply(&client, migrations_dir)
        }
    }
}

// ── Endpoint patching ────────────────────────────────────────────────────

/// Re-apply the remote functions with Docker-internal endpoints so that
/// SurrealDB (running inside the Docker network) can reach the SSP/scheduler
/// via container names instead of `localhost`.
fn patch_endpoints(surreal_port: u16, mode: &str) -> Result<()> {
    let endpoint = if mode == "cluster" {
        format!("http://scheduler:{}", SCHEDULER_PORT)
    } else {
        format!("http://ssp:{}", SSP_PORT)
    };
    let secret = "mysecret";

    let template = include_str!("functions_remote_singlenode.surql");
    let mut patched = template.to_string();
    patched = patched.replace("{{ENDPOINT}}", &endpoint);
    patched = patched.replace("{{SECRET}}", secret);

    // Also patch the unregister_view event (same replacement as schema_builder)
    let unregister_call = "let $result = mod::dbsp::unregister_view(<string>$before.id);";
    let unregister_http = format!(
        "let $payload = {{ id: <string>$before.id }};\n    let $result = http::post('{}/view/unregister', $payload, {{ \"Authorization\": \"Bearer {}\" }});",
        endpoint, secret
    );
    patched = patched.replace(unregister_call, &unregister_http);

    let client = SurrealClient::new(
        &format!("http://localhost:{}", surreal_port),
        "main",
        "main",
        "root",
        "root",
    );

    client.execute(&patched).context("Failed to patch endpoint functions")?;
    println!("{} Endpoints patched → {}", PREFIX, endpoint);
    Ok(())
}

// ── Docker helpers ──────────────────────────────────────────────────────────

fn docker(args: &[&str]) -> Result<()> {
    let output = Command::new("docker")
        .args(args)
        .output()
        .with_context(|| format!("Failed to run: docker {}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("docker {} failed: {}", args.join(" "), stderr.trim());
    }

    Ok(())
}

/// Spawn a background thread that tails container logs.
/// Returns a guard that kills the child process on drop.
struct LogTailGuard(Option<std::process::Child>);

impl Drop for LogTailGuard {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn spawn_log_tail(container: &str, label: &str) -> LogTailGuard {
    let child = Command::new("docker")
        .args(["logs", "-f", "--tail", "50", container])
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn();

    match child {
        Ok(c) => LogTailGuard(Some(c)),
        Err(e) => {
            eprintln!("{} Warning: Could not tail {} logs: {}", PREFIX, label, e);
            LogTailGuard(None)
        }
    }
}
