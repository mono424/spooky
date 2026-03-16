use anyhow::{bail, Context, Result};
use std::path::Path;
use std::io::IsTerminal;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::backend::{self, BackendDevConfig, BackendDevTypedConfig, ResolvedVersions, SpookyConfig, DEFAULT_CONFIG_PATH};
use crate::migrate;
use crate::schema_builder;
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

pub fn run(skip_migrations: bool, auto_apply_migrations: bool) -> Result<()> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();
    ctrlc::set_handler(move || {
        stop_clone.store(true, Ordering::SeqCst);
    })
    .context("Failed to set Ctrl+C handler")?;

    println!("{} Starting development environment...", PREFIX);

    // Read config from spooky.yml
    let config = backend::load_config(Path::new(DEFAULT_CONFIG_PATH));
    let mode = config.mode.clone().unwrap_or_else(|| "singlenode".to_string());
    let versions = ResolvedVersions::from_config(&config);
    let resolved = config.resolved_schema();
    let migrations_path = resolved.migrations.to_string_lossy().to_string();
    let migrations_path = migrations_path.as_str();
    println!("{} Mode: {}", PREFIX, mode);

    // Check for compose files
    let compose_file = format!("docker-compose.{}.yml", mode);
    if Path::new(&compose_file).exists() {
        println!("{} Found compose file: {}", PREFIX, compose_file);
        run_compose_mode(&compose_file, &mode, &config, &stop, skip_migrations, auto_apply_migrations, migrations_path)
    } else {
        println!("{} No compose file found — using direct Docker mode", PREFIX);
        run_direct_mode(&mode, &versions, &config, &stop, skip_migrations, auto_apply_migrations, migrations_path)
    }
}

// ── Direct Docker mode ──────────────────────────────────────────────────────

fn run_direct_mode(mode: &str, versions: &ResolvedVersions, config: &SpookyConfig, stop: &Arc<AtomicBool>, skip_migrations: bool, auto_apply_migrations: bool, migrations_path: &str) -> Result<()> {
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
    let surreal_data_dir = std::env::current_dir()
        .context("Failed to get current directory")?
        .join(".spooky/surrealdb_data");
    std::fs::create_dir_all(&surreal_data_dir).ok();
    let surreal_data_mount = format!("{}:/data", surreal_data_dir.display());

    docker(&[
        "run", "-d",
        "--name", SURREAL_CONTAINER,
        "--network", NETWORK_NAME,
        "--network-alias", "surrealdb",
        "-p", &format!("{}:8000", SURREAL_PORT),
        "-v", &surreal_data_mount,
        "-e", "SURREAL_USER=root",
        "-e", "SURREAL_PASS=root",
        "-e", "SURREAL_LOG=info",
        &surreal_image,
        "start",
        "--bind", "0.0.0.0:8000",
        "--allow-all",
        "--user", "root",
        "--pass", "root",
        "surrealkv:/data",
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
    if skip_migrations {
        println!("{} Phase 4: Skipping migrations (--skip-migrations).", PREFIX);
    } else {
        println!("{} Phase 4: Applying migrations...", PREFIX);
        apply_migrations(SURREAL_PORT, auto_apply_migrations, migrations_path)?;
    }

    if stop.load(Ordering::SeqCst) {
        return cleanup_direct(stop);
    }

    // Phase 4a: Apply internal Spooky schema (meta tables + events)
    println!("{} Phase 4a: Applying internal Spooky schema...", PREFIX);
    apply_internal_spooky_schema(SURREAL_PORT, mode)?;

    if stop.load(Ordering::SeqCst) {
        return cleanup_direct(stop);
    }

    // Phase 4b: Apply remote functions with Docker-internal endpoints
    println!("{} Phase 4b: Applying remote functions...", PREFIX);
    apply_remote_functions(SURREAL_PORT, mode)?;

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

    // Start the app dev server
    let app_dev = spawn_pnpm_dev_app();

    // Start backend dev commands
    let project_dir = std::env::current_dir().context("Failed to get current directory")?;
    let backend_devs = spawn_backend_dev_commands(config, &project_dir);

    // Wait for Ctrl+C
    while !stop.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(250));
    }

    // Stop backend dev commands, log tailers, and app dev server
    drop(backend_devs);
    drop(app_dev);
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

fn run_compose_mode(compose_file: &str, mode: &str, config: &SpookyConfig, stop: &Arc<AtomicBool>, skip_migrations: bool, auto_apply_migrations: bool, migrations_path: &str) -> Result<()> {
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
    if skip_migrations {
        println!("\n{} Phase 3: Skipping migrations (--skip-migrations).", PREFIX);
    } else {
        println!("\n{} Phase 3: Applying migrations...", PREFIX);
        apply_migrations(SURREAL_PORT, auto_apply_migrations, migrations_path)?;
    }

    if stop.load(Ordering::SeqCst) {
        return cleanup_compose(compose_file);
    }

    // Phase 3a: Apply internal Spooky schema (meta tables + events)
    println!("{} Phase 3a: Applying internal Spooky schema...", PREFIX);
    apply_internal_spooky_schema(SURREAL_PORT, mode)?;

    if stop.load(Ordering::SeqCst) {
        return cleanup_compose(compose_file);
    }

    // Phase 3b: Apply remote functions with Docker-internal endpoints
    println!("{} Phase 3b: Applying remote functions...", PREFIX);
    apply_remote_functions(SURREAL_PORT, mode)?;

    if stop.load(Ordering::SeqCst) {
        return cleanup_compose(compose_file);
    }

    // Phase 4: Start remaining services (foreground)
    println!("\n{} Phase 4: Starting remaining services...", PREFIX);
    println!("{} Press Ctrl+C to stop.\n", PREFIX);

    // Start the app dev server
    let app_dev = spawn_pnpm_dev_app();

    // Start backend dev commands
    let project_dir = std::env::current_dir().context("Failed to get current directory")?;
    let backend_devs = spawn_backend_dev_commands(config, &project_dir);

    let status = Command::new("docker")
        .args(["compose", "-f", compose_file, "up", "--remove-orphans"])
        .status()
        .context("Failed to run docker compose up")?;

    drop(backend_devs);
    drop(app_dev);

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

fn apply_migrations(port: u16, auto_apply: bool, migrations_path: &str) -> Result<()> {
    let migrations_dir = Path::new(migrations_path);
    if !migrations_dir.exists() {
        println!("{} No {}/ directory found, skipping.", PREFIX, migrations_path);
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

    if auto_apply {
        println!("{} Auto-applying migrations (--apply-migrations).", PREFIX);
    } else if !std::io::stdin().is_terminal() {
        println!("{} Non-TTY detected, auto-applying migrations.", PREFIX);
    } else {
        let options = vec![
            "Apply migrations",
            "Skip migrations (continue without applying)",
            "Quit",
        ];
        let choice = inquire::Select::new(
            &format!("{} pending migration(s) found. What would you like to do?", pending.len()),
            options,
        )
        .prompt()
        .unwrap_or("Quit");

        match choice {
            "Apply migrations" => {}
            "Skip migrations (continue without applying)" => {
                println!("{} Skipping migrations. Dev server will start without applying pending migrations.", PREFIX);
                return Ok(());
            }
            _ => bail!("User chose to quit."),
        }
    }

    match migrate::apply(&client, migrations_dir) {
        Ok(()) => Ok(()),
        Err(e) => {
            println!("{} Migration failed: {:#}", PREFIX, e);
            if auto_apply || !std::io::stdin().is_terminal() {
                println!("{} Auto-resetting database and retrying migrations.", PREFIX);
                println!("{} Resetting database and retrying...", PREFIX);
                client.reset_database()?;
                migrate::apply(&client, migrations_dir)
            } else {
                let options = vec![
                    "Reset database and retry",
                    "Skip migrations (continue without applying)",
                    "Quit",
                ];
                let choice = inquire::Select::new(
                    "Migration failed. What would you like to do?",
                    options,
                )
                .prompt()
                .unwrap_or("Quit");

                match choice {
                    "Reset database and retry" => {
                        println!("{} Resetting database and retrying...", PREFIX);
                        client.reset_database()?;
                        migrate::apply(&client, migrations_dir)
                    }
                    "Skip migrations (continue without applying)" => {
                        println!("{} Skipping migrations. Dev server will start without applying pending migrations.", PREFIX);
                        Ok(())
                    }
                    _ => bail!("User chose to quit."),
                }
            }
        }
    }
}

// ── Remote functions helper ─────────────────────────────────────────────────

/// Apply the remote functions with Docker-internal endpoints so that
/// SurrealDB (running inside the Docker network) can reach the SSP/scheduler
/// via container names instead of `localhost`.
fn apply_remote_functions(surreal_port: u16, mode: &str) -> Result<()> {
    let endpoint = if mode == "cluster" {
        format!("http://scheduler:{}", SCHEDULER_PORT)
    } else {
        format!("http://ssp:{}", SSP_PORT)
    };
    let secret = "mysecret";

    let functions_sql = schema_builder::build_remote_functions_schema(mode, &endpoint, secret);

    let client = SurrealClient::new(
        &format!("http://localhost:{}", surreal_port),
        "main",
        "main",
        "root",
        "root",
    );

    client.execute(&functions_sql).context("Failed to apply remote functions")?;
    println!("{} Remote functions applied → {}", PREFIX, endpoint);
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

fn spawn_pnpm_dev_app() -> LogTailGuard {
    println!("{} Starting app dev server (pnpm dev:app)...", PREFIX);
    let child = Command::new("pnpm")
        .args(["dev:app"])
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn();

    match child {
        Ok(c) => LogTailGuard(Some(c)),
        Err(e) => {
            eprintln!("{} Warning: Could not start pnpm dev:app: {}", PREFIX, e);
            LogTailGuard(None)
        }
    }
}

/// Apply the internal Spooky schema (meta tables + per-table events) so that
/// record versioning and DBSP ingest work after migrations are applied.
fn apply_internal_spooky_schema(surreal_port: u16, mode: &str) -> Result<()> {
    let config = backend::load_config(Path::new(DEFAULT_CONFIG_PATH));
    let resolved = config.resolved_schema();

    if !resolved.schema.exists() {
        println!("{} No schema file found at {:?}, skipping internal schema.", PREFIX, resolved.schema);
        return Ok(());
    }

    let endpoint = if mode == "cluster" {
        format!("http://scheduler:{}", SCHEDULER_PORT)
    } else {
        format!("http://ssp:{}", SSP_PORT)
    };
    let secret = "mysecret";

    let config_path = Path::new(DEFAULT_CONFIG_PATH);
    let config_path_ref = if config_path.exists() {
        Some(config_path)
    } else {
        None
    };

    let client = SurrealClient::new(
        &format!("http://localhost:{}", surreal_port),
        "main",
        "main",
        "root",
        "root",
    );

    migrate::apply_internal_schema(
        &client,
        &resolved.schema,
        config_path_ref,
        mode,
        Some(&endpoint),
        Some(secret),
    )
}

// ── Backend dev command helpers ──────────────────────────────────────────────

fn spawn_backend_dev_commands(config: &SpookyConfig, project_dir: &Path) -> Vec<LogTailGuard> {
    let mut guards = Vec::new();
    for (name, backend) in &config.backends {
        let dev_config = match &backend.dev {
            Some(cfg) => cfg,
            None => continue,
        };
        let label = format!("backend:{}", name);
        match dev_config {
            BackendDevConfig::Command(cmd) => {
                println!("{} Starting backend '{}': {}", PREFIX, name, cmd);
                guards.push(spawn_shell_command(cmd, project_dir, &label));
            }
            BackendDevConfig::Typed(BackendDevTypedConfig::Npm { script, workdir }) => {
                let cwd = resolve_workdir(project_dir, workdir.as_deref());
                println!("{} Starting backend '{}': pnpm run {}", PREFIX, name, script);
                guards.push(spawn_pnpm_script(script, &cwd, &label));
            }
            BackendDevConfig::Typed(BackendDevTypedConfig::Docker { file, workdir, port, env }) => {
                let cwd = resolve_workdir(project_dir, workdir.as_deref());
                println!("{} Starting backend '{}': docker build -f {}", PREFIX, name, file);
                guards.push(spawn_docker_dev(file, port.as_deref(), env, &cwd, name));
            }
            BackendDevConfig::Typed(BackendDevTypedConfig::Uv { script, workdir }) => {
                let cwd = resolve_workdir(project_dir, workdir.as_deref());
                println!("{} Starting backend '{}': uv run {}", PREFIX, name, script);
                guards.push(spawn_uv_script(script, &cwd, &label));
            }
        }
    }
    guards
}

fn resolve_workdir(project_dir: &Path, workdir: Option<&str>) -> std::path::PathBuf {
    match workdir {
        Some(dir) => project_dir.join(dir),
        None => project_dir.to_path_buf(),
    }
}

fn spawn_shell_command(cmd: &str, cwd: &Path, label: &str) -> LogTailGuard {
    let child = Command::new("sh")
        .args(["-c", cmd])
        .current_dir(cwd)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn();

    match child {
        Ok(c) => LogTailGuard(Some(c)),
        Err(e) => {
            eprintln!("{} Warning: Could not start {}: {}", PREFIX, label, e);
            LogTailGuard(None)
        }
    }
}

fn spawn_pnpm_script(script: &str, cwd: &Path, label: &str) -> LogTailGuard {
    let child = Command::new("pnpm")
        .args(["run", script])
        .current_dir(cwd)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn();

    match child {
        Ok(c) => LogTailGuard(Some(c)),
        Err(e) => {
            eprintln!("{} Warning: Could not start {}: {}", PREFIX, label, e);
            LogTailGuard(None)
        }
    }
}

fn spawn_uv_script(script: &str, cwd: &Path, label: &str) -> LogTailGuard {
    let child = Command::new("uv")
        .args(["run", script])
        .current_dir(cwd)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn();

    match child {
        Ok(c) => LogTailGuard(Some(c)),
        Err(e) => {
            eprintln!("{} Warning: Could not start {}: {}", PREFIX, label, e);
            LogTailGuard(None)
        }
    }
}

fn spawn_docker_dev(file: &str, port: Option<&str>, env: &[String], cwd: &Path, name: &str) -> LogTailGuard {
    let tag = format!("spooky-dev-{}", name);
    let container_name = format!("spooky-dev-backend-{}", name);

    // Build the image
    let build_status = Command::new("docker")
        .args(["build", "-f", file, "-t", &tag, "."])
        .current_dir(cwd)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();

    match build_status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!("{} Warning: docker build for '{}' exited with {}", PREFIX, name, s);
            return LogTailGuard(None);
        }
        Err(e) => {
            eprintln!("{} Warning: Could not run docker build for '{}': {}", PREFIX, name, e);
            return LogTailGuard(None);
        }
    }

    // Remove any stale container with the same name
    let _ = Command::new("docker").args(["rm", "-f", &container_name]).output();

    // Run the container
    let mut args = vec![
        "run".to_string(), "--rm".to_string(),
        "--name".to_string(), container_name,
        "--network".to_string(), NETWORK_NAME.to_string(),
    ];

    if let Some(p) = port {
        args.push("-p".to_string());
        args.push(p.to_string());
    }

    for e in env {
        args.push("-e".to_string());
        args.push(e.to_string());
    }

    args.push(tag);

    let child = Command::new("docker")
        .args(&args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn();

    match child {
        Ok(c) => LogTailGuard(Some(c)),
        Err(e) => {
            eprintln!("{} Warning: Could not start docker run for '{}': {}", PREFIX, name, e);
            LogTailGuard(None)
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
