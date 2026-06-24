use anyhow::{bail, Context, Result};
use std::io::{BufRead, BufReader, IsTerminal};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::backend::{self, BackendDevConfig, BackendDevTypedConfig, DeployEnv, DeployMode, HostingMode, ResolvedSurrealDb, ResolvedVersions, RuntimeSource, Sp00kyConfig, DEFAULT_CONFIG_PATH};
use crate::migrate;
use crate::port_check;
use crate::schema_builder::{self, SchemaBuilderConfig};
use crate::schema_diff;
use crate::schema_extract;
use crate::surreal_client::{MigrationDB, SurrealClient};

const PREFIX: &str = "[sp00ky dev]";

const NETWORK_NAME: &str = "sp00ky-dev-net";
const SURREAL_CONTAINER: &str = "sp00ky-dev-surrealdb";
const SSP_CONTAINER: &str = "sp00ky-dev-ssp";
const SCHEDULER_CONTAINER: &str = "sp00ky-dev-scheduler";

pub(crate) const SURREAL_PORT: u16 = 8666;
pub(crate) const SSP_PORT: u16 = 8667;
pub(crate) const SCHEDULER_PORT: u16 = 9667;
const HEALTH_MAX_RETRIES: u32 = 30;
const HEALTH_RETRY_INTERVAL: Duration = Duration::from_secs(2);

const INFRA_SERVICES_SINGLENODE: &[&str] = &["surrealdb", "aspire-dashboard"];
const INFRA_SERVICES_CLUSTER: &[&str] = &["surrealdb"];
const INFRA_SERVICES_SURREALISM: &[&str] = &["surrealdb"];

/// Returns the SurrealDB URL for the given config: either the external endpoint
/// or `http://localhost:{port}` for locally hosted instances.
pub(crate) fn surreal_connection_url(resolved: &ResolvedSurrealDb, local_port: u16) -> String {
    match &resolved.hosting {
        HostingMode::External => resolved.endpoint.clone().unwrap_or_else(|| {
            format!("http://localhost:{}", local_port)
        }),
        HostingMode::Cloud => format!("http://localhost:{}", local_port),
    }
}

/// Collect every host port `spky dev` will try to bind in the current mode.
/// Mirrors the gates around each `docker run -p` in `run_direct_mode` /
/// `run_compose_mode`: SurrealDB only when locally hosted, scheduler only in
/// cluster mode, plus any user `dev: { type: docker, port: "..." }` entries.
fn collect_dev_ports(
    config: &Sp00kyConfig,
    mode: &DeployMode,
    resolved_surreal: &ResolvedSurrealDb,
) -> Vec<(u16, String)> {
    let mut out: Vec<(u16, String)> = Vec::new();
    out.push((SSP_PORT, "ssp".to_string()));
    if resolved_surreal.hosting != HostingMode::External {
        out.push((SURREAL_PORT, "surrealdb".to_string()));
    }
    if *mode == DeployMode::Cluster {
        out.push((SCHEDULER_PORT, "scheduler".to_string()));
    }

    let mut collect_app = |label: &str, name: &str, dev: &Option<BackendDevConfig>| {
        if let Some(BackendDevConfig::Typed(BackendDevTypedConfig::Docker { port: Some(p), .. })) = dev {
            match port_check::parse_docker_host_port(p) {
                Some(host) => out.push((host, format!("{}:{}", label, name))),
                None => eprintln!(
                    "{} Warning: could not parse docker port spec '{}' for {} '{}', skipping pre-check for it",
                    PREFIX, p, label, name
                ),
            }
        }
    };

    if let Some((name, fe)) = config.frontend() {
        if fe.runs_in_dev() {
            collect_app("app", name, &fe.dev);
        }
    }
    for (name, app) in config.backends() {
        if !app.runs_in_dev() {
            continue; // cloudOnly: not started in dev, don't reserve its port
        }
        collect_app("app", name, &app.dev);
    }
    // Docker apps publish their ports directly (no BackendDevConfig).
    for (name, app) in config.docker_apps() {
        if !app.runs_in_dev() {
            continue;
        }
        for p in &app.ports {
            if let Some(host) = p.host_port() {
                out.push((host, format!("app:{}", name)));
            }
        }
    }

    out
}

// ── Public entry point ──────────────────────────────────────────────────────

pub fn run(skip_migrations: bool, auto_apply_migrations: bool, fix_checksums: bool, clean: bool, clean_db: bool) -> Result<()> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();
    ctrlc::set_handler(move || {
        stop_clone.store(true, Ordering::SeqCst);
    })
    .context("Failed to set Ctrl+C handler")?;

    println!("{} Starting development environment...", PREFIX);

    // Load config first so the port pre-check below is mode-aware and so we
    // don't wipe local state in `--clean`/`--clean-db` if the pre-check fails.
    let config = backend::load_config(Path::new(DEFAULT_CONFIG_PATH));
    let mode = config.mode.clone().unwrap_or(DeployMode::Singlenode);
    // Anchor any relative `version: { ssp: { path: ... } }` entries against
    // the project directory (where sp00ky.yml lives), not the user's cwd.
    // Otherwise `../../target/debug/ssp-server` from inside `example/`
    // would only work when invoked from that exact dir.
    let project_dir = std::env::current_dir().context("Failed to get current directory")?;
    let versions = ResolvedVersions::from_config_with_dir(&config, DeployEnv::Dev, &project_dir);
    let resolved = config.resolved_schema();
    let resolved_surreal = config.resolved_surrealdb();
    let migrations_path = resolved.migrations.to_string_lossy().to_string();
    let migrations_path = migrations_path.as_str();
    println!("{} Mode: {}", PREFIX, mode);

    // Pre-flight port check: bail before touching docker or local state if
    // any port we're about to bind is already taken.
    port_check::ensure_ports_free(
        collect_dev_ports(&config, &mode, &resolved_surreal),
        PREFIX,
    )?;

    // `--clean-db` implies `--clean`: wiping the DB while keeping SSP/
    // scheduler caches would leave them rebootstrapping into stale state.
    let clean_state = clean || clean_db;

    if clean_state || clean_db {
        let mut subs: Vec<&str> = Vec::new();
        if clean_state {
            subs.extend_from_slice(&["ssp_data", "scheduler_data"]);
        }
        if clean_db {
            // Stop the existing SurrealDB container first so the running
            // process doesn't hold the volume open while we delete it.
            // Best-effort: ignored if the container doesn't exist.
            let _ = docker(&["rm", "-f", SURREAL_CONTAINER]);
            subs.push("surrealdb_data");
        }

        for sub in subs {
            let path = project_dir.join(".sp00ky").join(sub);
            if path.exists() {
                std::fs::remove_dir_all(&path)
                    .with_context(|| format!("Failed to remove {}", path.display()))?;
                println!("{} --clean: removed {}", PREFIX, path.display());
            }
        }

        if !clean_db {
            println!("{} --clean: SurrealDB volume preserved (pass --clean-db for a full reset)", PREFIX);
        } else {
            println!("{} --clean-db: SurrealDB volume wiped — starting from an empty database", PREFIX);
        }
    }

    // Check for schema drift before starting infrastructure
    if !skip_migrations {
        println!("{} Checking for schema drift...", PREFIX);
        if let Err(e) = check_schema_drift(&config) {
            eprintln!("{} Warning: Schema drift check failed: {:#}", PREFIX, e);
            eprintln!("{} Continuing without drift check. Run `sp00ky migrate create` to check manually.", PREFIX);
        }
    }

    // Check for compose files
    let compose_file = format!("docker-compose.{}.yml", mode.to_string());
    if Path::new(&compose_file).exists() {
        println!("{} Found compose file: {}", PREFIX, compose_file);
        // Compose mode is driven by the YAML, not by `version.{ssp,scheduler}`.
        // A `path:` entry can't take effect there, so flag it loudly so the
        // user doesn't silently keep hitting the published image.
        if versions.ssp.is_local() || versions.scheduler.is_local() {
            eprintln!(
                "{} Warning: `version: {{ ssp/scheduler: {{ path: ... }} }}` is ignored in compose mode. The {} file controls those services. Either delete it (to use direct Docker mode with the path) or remove the path entry.",
                PREFIX, compose_file
            );
        }
        run_compose_mode(&compose_file, &mode, &config, &resolved_surreal, &stop, skip_migrations, auto_apply_migrations, fix_checksums, migrations_path)
    } else {
        println!("{} No compose file found — using direct Docker mode", PREFIX);
        run_direct_mode(&mode, &versions, &config, &resolved_surreal, &stop, skip_migrations, auto_apply_migrations, fix_checksums, migrations_path)
    }
}

// ── Schema drift detection ──────────────────────────────────────────────────

fn check_schema_drift(config: &Sp00kyConfig) -> Result<()> {
    let resolved = config.resolved_schema();
    let schema_path = &resolved.schema;
    let migrations_dir = &resolved.migrations;

    // No schema file → nothing to check
    if !schema_path.exists() {
        println!("{} No schema file found, skipping drift check.", PREFIX);
        return Ok(());
    }

    // Build the desired schema from source files
    let config_path = Path::new(DEFAULT_CONFIG_PATH);
    let builder_config = SchemaBuilderConfig {
        input_path: schema_path.clone(),
        config_path: if config_path.exists() { Some(config_path.to_path_buf()) } else { None },
        mode: config.mode.clone().unwrap_or(DeployMode::Singlenode),
        endpoint: None,
        secret: None,
        include_functions: false,
    };

    let new_schema_sql = schema_builder::build_server_schema(&builder_config)
        .context("Failed to build schema from source files")?;

    // Extract old (from migrations) and new (from source) schemas via ephemeral DB
    let (old_schema, new_schema) = schema_extract::extract_old_and_new_schemas(
        migrations_dir,
        &new_schema_sql,
    )
    .context("Failed to extract schemas for drift comparison")?;

    // Diff
    let diff = schema_diff::diff_schemas(&old_schema, &new_schema);

    if diff.is_empty() {
        println!("{} Schema is in sync.", PREFIX);
        return Ok(());
    }

    // Drift detected — show summary
    println!(
        "{} Schema drift detected: {} addition(s), {} removal(s), {} modification(s)",
        PREFIX,
        diff.added.len(),
        diff.removed.len(),
        diff.modified.len(),
    );
    diff.print_colored();

    // Non-TTY: warn and continue (matches existing pattern in apply_migrations)
    if !std::io::stdin().is_terminal() {
        println!(
            "{} Non-TTY detected, continuing with schema drift. Run `sp00ky migrate create` to generate a migration.",
            PREFIX,
        );
        return Ok(());
    }

    // Interactive prompt
    let options = vec![
        "Generate migration",
        "Continue anyway",
        "Abort",
    ];
    let choice = inquire::Select::new(
        "Schema drift detected. What would you like to do?",
        options,
    )
    .prompt()
    .unwrap_or("Abort");

    match choice {
        "Generate migration" => {
            let name = inquire::Text::new("Migration name:")
                .prompt()
                .context("Failed to read migration name")?;

            migrate::create(
                migrations_dir,
                &name,
                None,
                Some(&builder_config),
                None,
            )
            .context("Failed to create migration")?;

            println!("{} Migration created. It will be applied in the next step.", PREFIX);
        }
        "Continue anyway" => {
            println!(
                "{} Continuing with schema drift. Run `sp00ky migrate create` to generate a migration later.",
                PREFIX,
            );
        }
        _ => bail!("User chose to abort due to schema drift."),
    }

    Ok(())
}

// ── Direct Docker mode ──────────────────────────────────────────────────────

/// Collapses the SSP/scheduler networking matrix into named getters.
///
/// SurrealDB and the rest of the dev infrastructure stay containerized, so
/// each side has to choose between docker-network DNS (`surrealdb:8000`,
/// `scheduler:9667`, `ssp:8667`) and host-side mapped ports
/// (`localhost:{SURREAL,SSP,SCHEDULER}_PORT`). When SSP and scheduler split
/// across host/container, the container side reaches the host via
/// `host.docker.internal`. See the table in the implementation plan.
struct RuntimeUrls {
    ssp_local: bool,
    scheduler_local: bool,
}

impl RuntimeUrls {
    fn new(ssp_local: bool, scheduler_local: bool) -> Self {
        Self { ssp_local, scheduler_local }
    }

    fn ssp_db_url(&self) -> String {
        if self.ssp_local {
            format!("http://localhost:{}", SURREAL_PORT)
        } else {
            "http://surrealdb:8000".to_string()
        }
    }

    fn ssp_db_ws(&self) -> String {
        if self.ssp_local {
            format!("ws://localhost:{}", SURREAL_PORT)
        } else {
            "ws://surrealdb:8000".to_string()
        }
    }

    fn scheduler_db_url(&self) -> String {
        if self.scheduler_local {
            format!("http://localhost:{}", SURREAL_PORT)
        } else {
            "http://surrealdb:8000".to_string()
        }
    }

    fn scheduler_db_ws(&self) -> String {
        if self.scheduler_local {
            format!("ws://localhost:{}", SURREAL_PORT)
        } else {
            "ws://surrealdb:8000".to_string()
        }
    }

    /// Where the SSP reaches the scheduler (cluster mode only).
    fn ssp_scheduler_url(&self) -> String {
        match (self.ssp_local, self.scheduler_local) {
            // SSP host: localhost works whether the scheduler is host (direct)
            // or in a container (mapped port).
            (true, _) => format!("http://localhost:{}", SCHEDULER_PORT),
            // SSP container, scheduler host: container reaches host.
            (false, true) => format!("http://host.docker.internal:{}", SCHEDULER_PORT),
            // Both in containers: docker DNS.
            (false, false) => format!("http://scheduler:{}", SCHEDULER_PORT),
        }
    }

    /// Address the SSP advertises to the scheduler. The scheduler stores it
    /// and reuses it as the SSP endpoint for ingest forwarding.
    fn ssp_advertise(&self) -> String {
        match (self.ssp_local, self.scheduler_local) {
            // SSP host, scheduler host: localhost loops back.
            (true, true) => format!("localhost:{}", SSP_PORT),
            // SSP host, scheduler container: container side reaches host.
            (true, false) => format!("host.docker.internal:{}", SSP_PORT),
            // SSP container: existing behaviour, advertise the container alias.
            (false, _) => format!("{}:{}", SSP_CONTAINER, SSP_PORT),
        }
    }
}

fn run_direct_mode(mode: &DeployMode, versions: &ResolvedVersions, config: &Sp00kyConfig, resolved_surreal: &ResolvedSurrealDb, stop: &Arc<AtomicBool>, skip_migrations: bool, auto_apply_migrations: bool, fix_checksums: bool, migrations_path: &str) -> Result<()> {
    let surreal_image = versions.surrealdb_image();
    // Networking matrix between SSP and scheduler shifts based on whether
    // each runs in a container or on the host. RuntimeUrls collapses that
    // matrix into named getters so the launch blocks below stay readable.
    let urls = RuntimeUrls::new(versions.ssp.is_local(), versions.scheduler.is_local());

    // Clean up any stale resources from a previous run
    let _ = docker(&["rm", "-f", SURREAL_CONTAINER]);
    let _ = docker(&["rm", "-f", SSP_CONTAINER]);
    let _ = docker(&["rm", "-f", SCHEDULER_CONTAINER]);
    let _ = docker(&["network", "rm", NETWORK_NAME]);

    let use_local_surreal = resolved_surreal.hosting != HostingMode::External;
    let surreal_url = surreal_connection_url(resolved_surreal, SURREAL_PORT);

    // Phase 1: Create Docker network
    println!("\n{} Phase 1: Creating Docker network...", PREFIX);
    docker(&["network", "create", NETWORK_NAME])?;

    // Phase 2: Start SurrealDB (skip if using external instance)
    if use_local_surreal {
        println!("{} Phase 2: Starting SurrealDB...", PREFIX);
        let surreal_data_dir = std::env::current_dir()
            .context("Failed to get current directory")?
            .join(".sp00ky/surrealdb_data");
        std::fs::create_dir_all(&surreal_data_dir).ok();
        let surreal_data_mount = format!("{}:/data", surreal_data_dir.display());

        let surreal_user_env = format!("SURREAL_USER={}", resolved_surreal.username);
        let surreal_pass_env = format!("SURREAL_PASS={}", resolved_surreal.password);

        docker(&[
            "run", "-d",
            "--name", SURREAL_CONTAINER,
            "--network", NETWORK_NAME,
            "--network-alias", "surrealdb",
            "-p", &format!("{}:8000", SURREAL_PORT),
            "-v", &surreal_data_mount,
            "-e", &surreal_user_env,
            "-e", &surreal_pass_env,
            "-e", "SURREAL_LOG=info",
            "-e", "SURREAL_CAPS_ALLOW_EXPERIMENTAL=surrealism,files",
            &surreal_image,
            "start",
            "--bind", "0.0.0.0:8000",
            "--allow-all",
            "--user", &resolved_surreal.username,
            "--pass", &resolved_surreal.password,
            "surrealkv:/data",
        ])?;

        // Phase 3: Wait for health
        println!("{} Phase 3: Waiting for SurrealDB health...", PREFIX);
        wait_for_health(
            &format!("http://localhost:{}/health", SURREAL_PORT),
            HEALTH_MAX_RETRIES,
            HEALTH_RETRY_INTERVAL,
            stop,
            "SurrealDB",
        )?;
    } else {
        println!("{} Phase 2: Using external SurrealDB at {}", PREFIX, surreal_url);
    }

    if stop.load(Ordering::SeqCst) {
        return cleanup_direct(stop);
    }

    // Phase 3a: Ensure namespace + database exist. Idempotent (the SurrealQL
    // uses IF NOT EXISTS) so it's safe to run on every boot, and required
    // after a `--clean-db` wipe — without it the migration engine's first
    // write hits "Couldn't write to a read only transaction" (SurrealDB's
    // way of saying the target NS/DB is missing) and the internal-schema
    // apply later fails with "database '...' does not exist".
    if use_local_surreal {
        println!("{} Phase 3a: Ensuring namespace/database...", PREFIX);
        let bootstrap_client = SurrealClient::new(
            &surreal_url,
            &resolved_surreal.namespace,
            &resolved_surreal.database,
            &resolved_surreal.username,
            &resolved_surreal.password,
        );
        bootstrap_client
            .ensure_ns_db()
            .context("Failed to bootstrap SurrealDB namespace/database")?;
    }

    // Phase 4: Apply migrations
    if skip_migrations {
        println!("{} Phase 4: Skipping migrations (--skip-migrations).", PREFIX);
    } else {
        println!("{} Phase 4: Applying migrations...", PREFIX);
        apply_migrations(&surreal_url, auto_apply_migrations, fix_checksums, migrations_path, resolved_surreal)?;
    }

    if stop.load(Ordering::SeqCst) {
        return cleanup_direct(stop);
    }

    // Phase 4a: Apply internal Sp00ky schema (meta tables + events)
    println!("{} Phase 4a: Applying internal Sp00ky schema...", PREFIX);
    apply_internal_sp00ky_schema(&surreal_url, mode, versions, resolved_surreal)?;

    if stop.load(Ordering::SeqCst) {
        return cleanup_direct(stop);
    }

    // Phase 4b: Apply remote functions with Docker-internal endpoints
    println!("{} Phase 4b: Applying remote functions...", PREFIX);
    apply_remote_functions(&surreal_url, mode, versions, resolved_surreal)?;

    if stop.load(Ordering::SeqCst) {
        return cleanup_direct(stop);
    }

    // Resolved RUST_LOG for both scheduler and SSP. `logLevel:` in sp00ky.yml
    // (string or `{ dev, cloud }` map) overrides; default `info` matches the
    // pre-feature behavior so projects that don't opt in see no change.
    let dev_log = config.resolved_log_level(DeployEnv::Dev);
    let dev_log_env = format!("RUST_LOG={}", dev_log);

    // Phase 5 (cluster only): Start scheduler before SSP so SSP can register
    // `scheduler_guard` is the lifecycle handle: in Image mode it owns the
    // `docker logs -f` tail, in LocalBinary mode it owns the spawned host
    // process directly. Both kill on Drop.
    let scheduler_guard: Option<LogTailGuard>;
    if *mode == DeployMode::Cluster {
        // Persist the scheduler replica + WAL to the host so `--clean` can
        // wipe it and so it survives container restarts.
        let scheduler_data_dir = std::env::current_dir()
            .context("Failed to get current directory")?
            .join(".sp00ky/scheduler_data");
        std::fs::create_dir_all(&scheduler_data_dir).ok();

        scheduler_guard = match &versions.scheduler {
            RuntimeSource::Image(_) => {
                let scheduler_image = versions.scheduler_image()
                    .expect("scheduler_image is Some when RuntimeSource::Image");
                let scheduler_port_mapping = format!("{}:9667", SCHEDULER_PORT);
                let scheduler_db_url_env = format!("SPKY_DB_URL={}", urls.scheduler_db_url());
                let scheduler_db_ws_env = format!("SPKY_DB_WS={}", urls.scheduler_db_ws());
                let scheduler_ns_env = format!("SPKY_DB_NS={}", resolved_surreal.namespace);
                let scheduler_db_env = format!("SPKY_DB_NAME={}", resolved_surreal.database);
                let scheduler_user_env = format!("SPKY_DB_USER={}", resolved_surreal.username);
                let scheduler_pass_env = format!("SPKY_DB_PASS={}", resolved_surreal.password);
                let scheduler_data_mount = format!("{}:/data", scheduler_data_dir.display());

                println!("{} Phase 5: Starting scheduler (docker)...", PREFIX);
                docker(&[
                    "run", "-d",
                    "--name", SCHEDULER_CONTAINER,
                    "--network", NETWORK_NAME,
                    "--network-alias", "scheduler",
                    "-p", &scheduler_port_mapping,
                    "-v", &scheduler_data_mount,
                    "-e", &dev_log_env,
                    "-e", &scheduler_db_url_env,
                    "-e", &scheduler_db_ws_env,
                    "-e", &scheduler_ns_env,
                    "-e", &scheduler_db_env,
                    "-e", &scheduler_user_env,
                    "-e", &scheduler_pass_env,
                    "-e", "SPKY_AUTH_SECRET=mysecret",
                    // Default 300s makes records take 5 minutes to land
                    // in the replica/SSP — unusable in dev.
                    "-e", "SPKY_SNAPSHOT_UPDATE_INTERVAL_SECS=2",
                    &scheduler_image,
                ])?;

                // Wait for /health/ready, which only flips to 200 after the scheduler
                // finishes cloning the upstream SurrealDB into its replica. Without
                // this gate, SSP boots in Phase 6 against an empty replica and
                // computes wrong list_refs.
                println!("{} Waiting for scheduler to clone replica from SurrealDB...", PREFIX);
                wait_for_health(
                    &format!("http://localhost:{}/health/ready", SCHEDULER_PORT),
                    HEALTH_MAX_RETRIES,
                    HEALTH_RETRY_INTERVAL,
                    stop,
                    "Scheduler",
                )?;

                Some(spawn_log_tail(SCHEDULER_CONTAINER, "scheduler"))
            }
            RuntimeSource::LocalBinary(path) => {
                if !path.exists() {
                    bail!(
                        "Scheduler binary not found at {}.\n  Hint: run `cargo build -p scheduler` (or set version.dev.scheduler back to a Docker tag).",
                        path.display()
                    );
                }
                println!("{} Phase 5: Starting scheduler (host process: {})...", PREFIX, path.display());
                let prefix = format!("{}[scheduler]{}", infra_color("scheduler"), ANSI_RESET);
                let mut cmd = Command::new(path);
                // The scheduler defaults `replica_db_path: ./data/replica`
                // and `wal_path: ./data/event_wal.log` (config.rs:72,79),
                // both relative to cwd. Run from `.sp00ky/scheduler_data`
                // so the host paths land where `--clean` already wipes.
                cmd.current_dir(&scheduler_data_dir);
                cmd.env("RUST_LOG", &dev_log)
                    .env("SPKY_DB_URL", urls.scheduler_db_url())
                    .env("SPKY_DB_WS", urls.scheduler_db_ws())
                    .env("SPKY_DB_NS", &resolved_surreal.namespace)
                    .env("SPKY_DB_NAME", &resolved_surreal.database)
                    .env("SPKY_DB_USER", &resolved_surreal.username)
                    .env("SPKY_DB_PASS", &resolved_surreal.password)
                    .env("SPKY_AUTH_SECRET", "mysecret")
                    // Default 300s makes records take 5 minutes to land
                    // in the replica/SSP — unusable in dev.
                    .env("SPKY_SNAPSHOT_UPDATE_INTERVAL_SECS", "2");
                let guard = spawn_prefixed(&mut cmd, &prefix);

                // Spawned host process already streams its own stdio, so
                // we don't tail docker logs. Use the no-container variant
                // of `wait_for_health` so it doesn't bail on the missing
                // `sp00ky-dev-scheduler` container before the first probe.
                println!("{} Waiting for scheduler to clone replica from SurrealDB...", PREFIX);
                wait_for_health_with_container(
                    &format!("http://localhost:{}/health/ready", SCHEDULER_PORT),
                    HEALTH_MAX_RETRIES,
                    HEALTH_RETRY_INTERVAL,
                    stop,
                    "Scheduler",
                    false,
                )?;

                Some(guard)
            }
        };
    } else {
        scheduler_guard = None;
    }

    if stop.load(Ordering::SeqCst) {
        return cleanup_direct(stop);
    }

    // Phase 6: Start SSP
    println!("{} Phase 6: Starting SSP...", PREFIX);
    let data_dir = std::env::current_dir()
        .context("Failed to get current directory")?
        .join(".sp00ky/ssp_data");

    // Ensure data dir exists
    std::fs::create_dir_all(&data_dir).ok();

    // Build SPKY_JOB_CONFIG from backend apps with outbox method (mode-agnostic)
    let job_config_json = build_job_config_json(config);

    let ssp_guard: LogTailGuard = match &versions.ssp {
        RuntimeSource::Image(_) => {
            let ssp_image = versions.ssp_image()
                .expect("ssp_image is Some when RuntimeSource::Image");
            let port_mapping = format!("{}:8667", SSP_PORT);
            let data_mount_str = format!("{}:/data", data_dir.display());

            let ssp_db_url_env = format!("SPKY_DB_URL={}", urls.ssp_db_url());
            let ssp_db_ws_env = format!("SPKY_DB_WS={}", urls.ssp_db_ws());
            let ssp_ns_env = format!("SPKY_DB_NS={}", resolved_surreal.namespace);
            let ssp_db_env = format!("SPKY_DB_NAME={}", resolved_surreal.database);
            let ssp_user_env = format!("SPKY_DB_USER={}", resolved_surreal.username);
            let ssp_pass_env = format!("SPKY_DB_PASS={}", resolved_surreal.password);
            let scheduler_url_env = format!("SPKY_SCHEDULER_URL={}", urls.ssp_scheduler_url());
            let advertise_addr_env = format!("SPKY_SSP_ADVERTISE_ADDR={}", urls.ssp_advertise());
            let job_config_env = format!("SPKY_JOB_CONFIG={}", job_config_json);
            let ref_mode_env = format!("SPKY_SSP_REF_MODE={}", config.resolved_ref_mode().as_str());
            let anon_live_env = format!(
                "SPKY_SSP_ANON_LIVE_QUERIES={}",
                if config.resolved_anonymous_live_queries() { "1" } else { "0" }
            );

            let mut ssp_args = vec![
                "run", "-d",
                "--name", SSP_CONTAINER,
                "--network", NETWORK_NAME,
                "--network-alias", "ssp",
                "-p", &port_mapping,
                "-e", &dev_log_env,
                "-e", &ssp_db_url_env,
                "-e", &ssp_db_ws_env,
                "-e", &ssp_ns_env,
                "-e", &ssp_db_env,
                "-e", &ssp_user_env,
                "-e", &ssp_pass_env,
                "-e", "SPKY_AUTH_SECRET=mysecret",
                "-e", &job_config_env,
                "-e", &ref_mode_env,
                "-e", &anon_live_env,
            ];

            if *mode == DeployMode::Cluster {
                ssp_args.extend(["-e", &scheduler_url_env]);
                ssp_args.extend(["-e", "SPKY_SSP_ID=ssp-1"]);
                ssp_args.extend(["-e", &advertise_addr_env]);
            }

            ssp_args.extend(["-v", &data_mount_str]);
            ssp_args.push(&ssp_image);

            docker(&ssp_args)?;
            spawn_log_tail(SSP_CONTAINER, "ssp")
        }
        RuntimeSource::LocalBinary(path) => {
            if !path.exists() {
                bail!(
                    "SSP binary not found at {}.\n  Hint: run `cargo build -p ssp-server` (or set version.dev.ssp back to a Docker tag).",
                    path.display()
                );
            }
            println!("{} Starting SSP (host process: {})...", PREFIX, path.display());
            let prefix = format!("{}[ssp]{}", infra_color("ssp"), ANSI_RESET);
            let mut cmd = Command::new(path);
            cmd.current_dir(&data_dir);
            cmd.env("RUST_LOG", &dev_log)
                .env("SPKY_DB_URL", urls.ssp_db_url())
                .env("SPKY_DB_WS", urls.ssp_db_ws())
                .env("SPKY_DB_NS", &resolved_surreal.namespace)
                .env("SPKY_DB_NAME", &resolved_surreal.database)
                .env("SPKY_DB_USER", &resolved_surreal.username)
                .env("SPKY_DB_PASS", &resolved_surreal.password)
                .env("SPKY_AUTH_SECRET", "mysecret")
                .env("SPKY_JOB_CONFIG", &job_config_json)
                .env("SPKY_SSP_REF_MODE", config.resolved_ref_mode().as_str())
                .env(
                    "SPKY_SSP_ANON_LIVE_QUERIES",
                    if config.resolved_anonymous_live_queries() { "1" } else { "0" },
                )
                // The container Dockerfile binds 0.0.0.0:8667; on host we
                // need the same port reachable from frontend dev servers
                // and (optionally) the docker-side scheduler.
                .env("SPKY_SSP_LISTEN_ADDR", format!("0.0.0.0:{}", SSP_PORT));
            if *mode == DeployMode::Cluster {
                cmd.env("SPKY_SCHEDULER_URL", urls.ssp_scheduler_url())
                    .env("SPKY_SSP_ID", "ssp-1")
                    .env("SPKY_SSP_ADVERTISE_ADDR", urls.ssp_advertise());
            }
            spawn_prefixed(&mut cmd, &prefix)
        }
    };

    // Ready!
    println!("\n{} Development environment ready!", PREFIX);
    println!("{} SurrealDB:  http://localhost:{}", PREFIX, SURREAL_PORT);
    println!("{} SSP:        http://localhost:{}", PREFIX, SSP_PORT);
    if *mode == DeployMode::Cluster {
        println!("{} Scheduler:  http://localhost:{}", PREFIX, SCHEDULER_PORT);
    }
    println!("{} Press Ctrl+C to stop.\n", PREFIX);

    // Tail logs from infra containers (SurrealDB always; SSP is already
    // captured inside `ssp_guard`).
    let surreal_log = spawn_log_tail(SURREAL_CONTAINER, "surrealdb");

    // Start app dev servers (frontend + backends)
    let project_dir = std::env::current_dir().context("Failed to get current directory")?;
    let app_dev = spawn_frontend_dev(config, &project_dir, resolved_surreal, mode);
    let backend_devs = spawn_backend_dev_commands(config, &project_dir, resolved_surreal, mode);
    let docker_devs = spawn_docker_app_devs(config, &project_dir, resolved_surreal, mode);

    // Wait for Ctrl+C
    while !stop.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(250));
    }

    // Stop backend dev commands, log tailers, and app dev server
    drop(docker_devs);
    drop(backend_devs);
    drop(app_dev);
    drop(surreal_log);
    drop(ssp_guard);
    drop(scheduler_guard);

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

    println!("{} Cleaned up. Goodbye! 👻", PREFIX);
    Ok(())
}

// ── Compose mode ────────────────────────────────────────────────────────────

fn run_compose_mode(compose_file: &str, mode: &DeployMode, config: &Sp00kyConfig, resolved_surreal: &ResolvedSurrealDb, stop: &Arc<AtomicBool>, skip_migrations: bool, auto_apply_migrations: bool, fix_checksums: bool, migrations_path: &str) -> Result<()> {
    let use_local_surreal = resolved_surreal.hosting != HostingMode::External;
    let surreal_url = surreal_connection_url(resolved_surreal, SURREAL_PORT);

    let infra_services: &[&str] = match mode {
        DeployMode::Cluster => INFRA_SERVICES_CLUSTER,
        DeployMode::Surrealism => INFRA_SERVICES_SURREALISM,
        _ => INFRA_SERVICES_SINGLENODE,
    };

    // Phase 1: Start infrastructure (filter out surrealdb if external)
    let infra_services: Vec<&str> = if use_local_surreal {
        infra_services.to_vec()
    } else {
        infra_services.iter().copied().filter(|s| *s != "surrealdb").collect()
    };

    if !infra_services.is_empty() {
        println!(
            "\n{} Phase 1: Starting infrastructure ({})...",
            PREFIX,
            infra_services.join(", ")
        );
        let mut args = vec![
            "compose", "-f", compose_file, "up", "-d", "--remove-orphans",
        ];
        for svc in &infra_services {
            args.push(svc);
        }
        docker(&args)?;
    } else {
        println!("\n{} Phase 1: Using external SurrealDB, skipping local infrastructure.", PREFIX);
    }

    if stop.load(Ordering::SeqCst) {
        return cleanup_compose(compose_file);
    }

    // Phase 2: Wait for SurrealDB health
    if use_local_surreal {
        println!("\n{} Phase 2: Waiting for SurrealDB health...", PREFIX);
        wait_for_health_with_container(
            &format!("http://localhost:{}/health", SURREAL_PORT),
            HEALTH_MAX_RETRIES,
            HEALTH_RETRY_INTERVAL,
            stop,
            "SurrealDB",
            false,
        )?;
    } else {
        println!("\n{} Phase 2: Using external SurrealDB at {}", PREFIX, surreal_url);
    }

    if stop.load(Ordering::SeqCst) {
        return cleanup_compose(compose_file);
    }

    // Phase 3: Apply migrations
    if skip_migrations {
        println!("\n{} Phase 3: Skipping migrations (--skip-migrations).", PREFIX);
    } else {
        println!("\n{} Phase 3: Applying migrations...", PREFIX);
        apply_migrations(&surreal_url, auto_apply_migrations, fix_checksums, migrations_path, resolved_surreal)?;
    }

    if stop.load(Ordering::SeqCst) {
        return cleanup_compose(compose_file);
    }

    // Phase 3a: Apply internal Sp00ky schema (meta tables + events)
    println!("{} Phase 3a: Applying internal Sp00ky schema...", PREFIX);
    // Compose mode launches both services in docker per the YAML, so the
    // SurrealDB-side endpoints stay at the docker-DNS aliases. Default
    // versions (Image-variants) selects exactly that.
    let compose_versions = ResolvedVersions::default();
    apply_internal_sp00ky_schema(&surreal_url, mode, &compose_versions, resolved_surreal)?;

    if stop.load(Ordering::SeqCst) {
        return cleanup_compose(compose_file);
    }

    // Phase 3b: Apply remote functions with Docker-internal endpoints
    println!("{} Phase 3b: Applying remote functions...", PREFIX);
    apply_remote_functions(&surreal_url, mode, &compose_versions, resolved_surreal)?;

    if stop.load(Ordering::SeqCst) {
        return cleanup_compose(compose_file);
    }

    // Phase 4: Start remaining services (foreground)
    println!("\n{} Phase 4: Starting remaining services...", PREFIX);
    println!("{} Press Ctrl+C to stop.\n", PREFIX);

    // Start app dev servers (frontend + backends)
    let project_dir = std::env::current_dir().context("Failed to get current directory")?;
    let app_dev = spawn_frontend_dev(config, &project_dir, resolved_surreal, mode);
    let backend_devs = spawn_backend_dev_commands(config, &project_dir, resolved_surreal, mode);
    let docker_devs = spawn_docker_app_devs(config, &project_dir, resolved_surreal, mode);

    let status = Command::new("docker")
        .args(["compose", "-f", compose_file, "up", "--build", "--remove-orphans"])
        .status()
        .context("Failed to run docker compose up")?;

    drop(docker_devs);
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
    println!("{} Cleaned up. Goodbye! 👻", PREFIX);
    Ok(())
}

// ── Health checking ─────────────────────────────────────────────────────────

fn wait_for_health(
    url: &str,
    max_retries: u32,
    interval: Duration,
    stop: &Arc<AtomicBool>,
    service_name: &str,
) -> Result<()> {
    wait_for_health_with_container(url, max_retries, interval, stop, service_name, true)
}

fn wait_for_health_with_container(
    url: &str,
    max_retries: u32,
    interval: Duration,
    stop: &Arc<AtomicBool>,
    service_name: &str,
    check_container: bool,
) -> Result<()> {
    // Try to infer container name from service name for liveness checks (direct mode only)
    let container_name = if check_container {
        match service_name {
            "SurrealDB" => Some(SURREAL_CONTAINER),
            "Scheduler" => Some(SCHEDULER_CONTAINER),
            "SSP" => Some(SSP_CONTAINER),
            _ => None,
        }
    } else {
        None
    };

    for attempt in 1..=max_retries {
        if stop.load(Ordering::SeqCst) {
            bail!("Interrupted while waiting for {}", service_name);
        }

        // Check if the container is still running (fail fast on crash)
        if let Some(name) = container_name {
            if !is_container_running(name) {
                // Print last logs to help diagnose
                let _ = print_container_logs(name, 20);
                bail!("{} container '{}' exited unexpectedly. Check logs above.", service_name, name);
            }
        }

        match ureq::get(url).timeout(Duration::from_secs(5)).call() {
            Ok(resp) if resp.status() == 200 => {
                println!("{} {} is ready.", PREFIX, service_name);
                return Ok(());
            }
            _ => {
                println!(
                    "{} Waiting for {}... ({}/{})",
                    PREFIX, service_name, attempt, max_retries
                );
                thread::sleep(interval);
            }
        }
    }

    // Print logs on timeout to help diagnose
    if let Some(name) = container_name {
        let _ = print_container_logs(name, 30);
    }

    bail!(
        "{} did not become ready after {} attempts",
        service_name, max_retries
    );
}

/// Check if a Docker container is currently running
fn is_container_running(name: &str) -> bool {
    Command::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", name])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

/// Print the last N lines of a container's logs
fn print_container_logs(name: &str, tail: u32) -> Result<()> {
    let output = Command::new("docker")
        .args(["logs", "--tail", &tail.to_string(), name])
        .output()
        .context("Failed to get container logs")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    println!("\n{} --- Last {} log lines from {} ---", PREFIX, tail, name);
    if !stdout.is_empty() {
        print!("{}", stdout);
    }
    if !stderr.is_empty() {
        eprint!("{}", stderr);
    }
    println!("{} --- End of {} logs ---\n", PREFIX, name);
    Ok(())
}

// ── Migration helper ────────────────────────────────────────────────────────

fn apply_migrations(surreal_url: &str, auto_apply: bool, fix_checksums: bool, migrations_path: &str, resolved_surreal: &ResolvedSurrealDb) -> Result<()> {
    use crate::migration::{self, MigrationState};

    let migrations_dir = Path::new(migrations_path);
    if !migrations_dir.exists() {
        println!("{} No {}/ directory found, skipping.", PREFIX, migrations_path);
        return Ok(());
    }

    let sp00ky_config = backend::load_config(Path::new(DEFAULT_CONFIG_PATH));

    // Build engine for user migrations only (internal schema + remote functions
    // are separate phases in the dev flow with stop-checks between them).
    let ctx = migration::MigrationContext {
        environment: migration::MigrationEnvironment::Dev,
        project_dir: std::env::current_dir().context("Failed to get current directory")?,
        migrations_dir: migrations_dir.to_path_buf(),
        url: surreal_url.to_string(),
        namespace: resolved_surreal.namespace.clone(),
        database: resolved_surreal.database.clone(),
        username: "root".to_string(),
        password: "root".to_string(),
        surrealkit_binary: sp00ky_config.resolved_surrealkit_binary(),
        internal_schema: None,
        remote_functions: None,
        // Dev vault secrets for {{KEY}} substitution in user migrations (e.g. the
        // EdDSA signing key for the `account` access). Empty / not logged in ->
        // migrations apply verbatim (see LegacyEngine::apply).
        secrets: Some(crate::cloud::load_vault_envs_for_dev()),
    };

    let engine = migration::create_engine(ctx)?;

    // Fix checksums first if requested
    if fix_checksums {
        if let Err(e) = engine.fix(true) {
            eprintln!("{} \x1b[33mWARNING: checksum fix failed: {:#}\x1b[0m", PREFIX, e);
        }
    }

    // Check for pending migrations
    let statuses = match engine.status() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{} \x1b[31mFailed to check migration status: {:#}\x1b[0m", PREFIX, e);
            return Ok(());
        }
    };

    // Report drift warnings
    for info in &statuses {
        if info.state == MigrationState::Drift {
            let detail = info.detail.as_deref().unwrap_or("");
            println!("{} \x1b[33mWARNING: Drift on {}_{}: {}\x1b[0m", PREFIX, info.id, info.name, detail);
        }
    }

    let pending: Vec<_> = statuses.iter().filter(|s| s.state == MigrationState::Pending).collect();

    if pending.is_empty() {
        println!("{} No pending migrations.", PREFIX);
        return Ok(());
    }

    println!("{} Found {} pending migration(s):", PREFIX, pending.len());
    for m in &pending {
        println!("  - {}_{}", m.id, m.name);
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

    match engine.apply() {
        Ok(_) => Ok(()),
        Err(e) => {
            println!("{} \x1b[31mMigration failed:\x1b[0m {:#}", PREFIX, e);

            // Reset-and-retry uses SurrealClient directly (dev-only escape hatch)
            let client = SurrealClient::new(
                surreal_url,
                &resolved_surreal.namespace,
                &resolved_surreal.database,
                "root",
                "root",
            );

            if auto_apply || !std::io::stdin().is_terminal() {
                println!("{} Auto-resetting database and retrying migrations.", PREFIX);
                client.reset_database()?;
                engine.apply().map(|_| ())
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
                        engine.apply().map(|_| ())
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

/// Pick the URL SurrealDB (in its container) should use to call out to
/// the SSP/scheduler. Container DNS aliases when the target also runs in
/// docker; `host.docker.internal` when it's a host process.
fn surreal_to_target_endpoint(mode: &DeployMode, versions: &ResolvedVersions) -> String {
    if *mode == DeployMode::Cluster {
        if versions.scheduler.is_local() {
            format!("http://host.docker.internal:{}", SCHEDULER_PORT)
        } else {
            format!("http://scheduler:{}", SCHEDULER_PORT)
        }
    } else {
        if versions.ssp.is_local() {
            format!("http://host.docker.internal:{}", SSP_PORT)
        } else {
            format!("http://ssp:{}", SSP_PORT)
        }
    }
}

/// Apply the remote functions with Docker-internal endpoints so that
/// SurrealDB (running inside the Docker network) can reach the SSP/scheduler
/// via container names instead of `localhost`.
fn apply_remote_functions(surreal_url: &str, mode: &DeployMode, versions: &ResolvedVersions, resolved_surreal: &ResolvedSurrealDb) -> Result<()> {
    let endpoint = surreal_to_target_endpoint(mode, versions);
    let secret = "mysecret";

    let functions_sql = schema_builder::build_remote_functions_schema(mode, &endpoint, secret);

    let client = SurrealClient::new(
        surreal_url,
        &resolved_surreal.namespace,
        &resolved_surreal.database,
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

/// Build SPKY_JOB_CONFIG JSON from backend apps with outbox methods.
/// Uses baseUrl from sp00ky.yml for dev mode (Docker-internal URLs use host.docker.internal).
fn build_job_config_json(config: &Sp00kyConfig) -> String {
    let mut entries = Vec::new();
    for (name, app) in config.backends() {
        if !app.runs_in_dev() {
            continue; // cloudOnly backend isn't running in dev; don't route jobs to it
        }
        let method = match &app.method {
            Some(m) => m,
            None => continue,
        };
        let base_url = match &app.base_url {
            Some(u) => u.clone(),
            None => continue,
        };
        let table = match &method.table {
            Some(t) => t.clone(),
            None => continue,
        };
        let auth_token = app.auth.as_ref().and_then(|a| a.token.clone());
        let timeout = app.deploy.as_ref().and_then(|d| d.timeout);
        let timeout_overridable = app.deploy.as_ref()
            .and_then(|d| d.timeout_overridable)
            .unwrap_or(false);

        entries.push(serde_json::json!({
            "name": name,
            "table": table,
            "base_url": base_url,
            "auth_token": auth_token,
            "timeout": timeout,
            "timeout_overridable": timeout_overridable,
        }));
    }
    serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string())
}

/// Build the auto-injected SPKY_* environment variables for dev mode.
fn build_spky_dev_vars(resolved_surreal: &ResolvedSurrealDb, mode: &DeployMode) -> Vec<(String, String)> {
    let mut vars = vec![
        ("SPKY_ENV".into(), "dev".into()),
        ("SPKY_DB_URL".into(), format!("http://localhost:{}", SURREAL_PORT)),
        ("SPKY_DB_WS".into(), format!("ws://localhost:{}", SURREAL_PORT)),
        ("SPKY_DB_NS".into(), resolved_surreal.namespace.clone()),
        ("SPKY_DB_NAME".into(), resolved_surreal.database.clone()),
        ("SPKY_DB_USER".into(), resolved_surreal.username.clone()),
        ("SPKY_DB_PASS".into(), resolved_surreal.password.clone()),
        ("SPKY_SSP_ADDR".into(), format!("localhost:{}", SSP_PORT)),
    ];
    if *mode == DeployMode::Cluster {
        vars.push(("SPKY_SCHEDULER_URL".into(), format!("http://localhost:{}", SCHEDULER_PORT)));
    }
    vars
}

/// Merge SPKY auto-injected vars with user-provided vars. User vars take precedence.
fn merge_spky_with_user_env(spky_vars: &[(String, String)], user_vars: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut merged = std::collections::BTreeMap::new();
    // SPKY vars first (base)
    for (k, v) in spky_vars {
        merged.insert(k.clone(), v.clone());
    }
    // User vars override
    for (k, v) in user_vars {
        merged.insert(k, v);
    }
    merged.into_iter().collect()
}

/// Warn if a frontend app uses vault without a whitelist.
fn warn_frontend_vault_no_whitelist(name: &str, env: &Option<backend::EnvConfig>) {
    if let Some(backend::EnvConfig::Source(backend::EnvSource::Str(s))) = env {
        if s == "vault" {
            eprintln!("  \x1b[33mwarning\x1b[0m: Frontend app '{}' uses vault without a whitelist. Consider using vault: [KEY1, KEY2] to avoid exposing all secrets to the frontend.", name);
        }
    }
}

const APP_COLOR: &str = "\x1b[97m"; // bright white

fn spawn_pnpm_dev_app(script: &str, envs: Vec<(String, String)>) -> LogTailGuard {
    let prefix = format!("{}[app]{}", APP_COLOR, ANSI_RESET);
    println!("{} Starting: pnpm {}", prefix, script);
    spawn_prefixed(
        Command::new("pnpm").args([script]).envs(envs),
        &prefix,
    )
}

/// Start the frontend app dev server from the apps config.
fn spawn_frontend_dev(config: &Sp00kyConfig, project_dir: &Path, resolved_surreal: &ResolvedSurrealDb, mode: &DeployMode) -> LogTailGuard {
    if let Some((name, frontend)) = config.frontend().filter(|(_, fe)| fe.runs_in_dev()) {
        warn_frontend_vault_no_whitelist(name, &frontend.env);
        let spky_vars = build_spky_dev_vars(resolved_surreal, mode);
        let user_envs = resolve_env_for_dev(&frontend.env, project_dir);
        let envs = merge_spky_with_user_env(&spky_vars, user_envs);
        // Use the same dev config dispatch as backends
        if let Some(ref dev_config) = frontend.dev {
            let prefix = format!("{}[app]{}", APP_COLOR, ANSI_RESET);
            match dev_config {
                BackendDevConfig::Command(cmd) => {
                    println!("{} Starting: {}", prefix, cmd);
                    return spawn_prefixed(
                        Command::new("sh").args(["-c", cmd.as_str()]).current_dir(project_dir).envs(envs),
                        &prefix,
                    );
                }
                BackendDevConfig::Typed(BackendDevTypedConfig::Npm { script, workdir }) => {
                    let cwd = resolve_workdir(project_dir, workdir.as_deref());
                    println!("{} Starting: pnpm run {}", prefix, script);
                    return spawn_prefixed(
                        Command::new("pnpm").args(["run", script]).current_dir(cwd).envs(envs),
                        &prefix,
                    );
                }
                BackendDevConfig::Typed(BackendDevTypedConfig::Docker { file, workdir, port }) => {
                    let cwd = resolve_workdir(project_dir, workdir.as_deref());
                    println!("{} Building: docker build -f {}", prefix, file);
                    return spawn_docker_dev(file, port.as_deref(), &envs, &cwd, "frontend", &prefix);
                }
                BackendDevConfig::Typed(BackendDevTypedConfig::Uv { script, workdir }) => {
                    let cwd = resolve_workdir(project_dir, workdir.as_deref());
                    println!("{} Starting: uv run {}", prefix, script);
                    return spawn_prefixed(
                        Command::new("uv").args(["run", script]).current_dir(cwd).envs(envs),
                        &prefix,
                    );
                }
            }
        }
        // Fallback: no dev config — try pnpm dev:app
        return spawn_pnpm_dev_app("dev:app", envs);
    }
    // No frontend app defined — try default pnpm dev:app
    spawn_pnpm_dev_app("dev:app", Vec::new())
}

/// Apply the internal Sp00ky schema (meta tables + per-table events) so that
/// record versioning and DBSP ingest work after migrations are applied.
fn apply_internal_sp00ky_schema(surreal_url: &str, mode: &DeployMode, versions: &ResolvedVersions, resolved_surreal: &ResolvedSurrealDb) -> Result<()> {
    let config = backend::load_config(Path::new(DEFAULT_CONFIG_PATH));
    let resolved = config.resolved_schema();

    if !resolved.schema.exists() {
        println!("{} No schema file found at {:?}, skipping internal schema.", PREFIX, resolved.schema);
        return Ok(());
    }

    let endpoint = surreal_to_target_endpoint(mode, versions);
    let secret = "mysecret";

    let config_path = Path::new(DEFAULT_CONFIG_PATH);
    let config_path_ref = if config_path.exists() {
        Some(config_path)
    } else {
        None
    };

    let client = SurrealClient::new(
        surreal_url,
        &resolved_surreal.namespace,
        &resolved_surreal.database,
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

/// ANSI color codes cycled across backends for distinguishable output.
const BACKEND_COLORS: &[&str] = &[
    "\x1b[36m",  // cyan
    "\x1b[33m",  // yellow
    "\x1b[35m",  // magenta
    "\x1b[32m",  // green
    "\x1b[34m",  // blue
    "\x1b[91m",  // bright red
    "\x1b[96m",  // bright cyan
    "\x1b[93m",  // bright yellow
    "\x1b[95m",  // bright magenta
    "\x1b[92m",  // bright green
];
const ANSI_RESET: &str = "\x1b[0m";

fn spawn_backend_dev_commands(config: &Sp00kyConfig, project_dir: &Path, resolved_surreal: &ResolvedSurrealDb, mode: &DeployMode) -> Vec<LogTailGuard> {
    let spky_vars = build_spky_dev_vars(resolved_surreal, mode);
    let mut guards = Vec::new();
    let mut color_idx = 0;
    for (name, app) in config.backends() {
        if !app.runs_in_dev() {
            continue; // cloudOnly: deployed but not started by `spky dev`
        }
        let dev_config = match &app.dev {
            Some(cfg) => cfg,
            None => continue,
        };
        let color = BACKEND_COLORS[color_idx % BACKEND_COLORS.len()];
        color_idx += 1;
        let prefix = format!("{}[app.{}.dev]{}", color, name, ANSI_RESET);
        let user_envs = resolve_env_for_dev(&app.env, project_dir);
        let envs = merge_spky_with_user_env(&spky_vars, user_envs);
        match dev_config {
            BackendDevConfig::Command(cmd) => {
                println!("{} Starting: {}", prefix, cmd);
                guards.push(spawn_prefixed(
                    Command::new("sh").args(["-c", cmd]).current_dir(project_dir).envs(envs),
                    &prefix,
                ));
            }
            BackendDevConfig::Typed(BackendDevTypedConfig::Npm { script, workdir }) => {
                let cwd = resolve_workdir(project_dir, workdir.as_deref());
                println!("{} Starting: pnpm run {}", prefix, script);
                guards.push(spawn_prefixed(
                    Command::new("pnpm").args(["run", script]).current_dir(cwd).envs(envs),
                    &prefix,
                ));
            }
            BackendDevConfig::Typed(BackendDevTypedConfig::Docker { file, workdir, port }) => {
                let cwd = resolve_workdir(project_dir, workdir.as_deref());
                println!("{} Building: docker build -f {}", prefix, file);
                guards.push(spawn_docker_dev(file, port.as_deref(), &envs, &cwd, name, &prefix));
            }
            BackendDevConfig::Typed(BackendDevTypedConfig::Uv { script, workdir }) => {
                let cwd = resolve_workdir(project_dir, workdir.as_deref());
                println!("{} Starting: uv run {}", prefix, script);
                guards.push(spawn_prefixed(
                    Command::new("uv").args(["run", script]).current_dir(cwd).envs(envs),
                    &prefix,
                ));
            }
        }
    }
    guards
}

/// Start each `type: docker` app (scope all or devOnly) by running its prebuilt
/// image in the foreground: `docker run --rm --name sp00ky-dev-<key> --network
/// <net> [-p <publish>]… [-e K=V]… <image> <args…>`. The returned LogTailGuard
/// kills `docker run` on Ctrl-C and `--rm` removes the container — same teardown
/// as the raw-command backend path.
fn spawn_docker_app_devs(config: &Sp00kyConfig, project_dir: &Path, resolved_surreal: &ResolvedSurrealDb, mode: &DeployMode) -> Vec<LogTailGuard> {
    let spky_vars = build_spky_dev_vars(resolved_surreal, mode);
    let mut guards = Vec::new();
    let mut color_idx = 0;
    for (name, app) in config.docker_apps() {
        if !app.runs_in_dev() {
            continue; // cloudOnly: deployed but not started by `spky dev`
        }
        let image = match &app.image {
            Some(i) => i,
            None => continue, // validation already requires it; defensive
        };
        let color = BACKEND_COLORS[color_idx % BACKEND_COLORS.len()];
        color_idx += 1;
        let prefix = format!("{}[app.{}.docker]{}", color, name, ANSI_RESET);
        let container = format!("sp00ky-dev-{}", name);

        // Clear any stale container from a previous (hard-killed) run.
        let _ = docker(&["rm", "-f", &container]);

        let user_envs = resolve_env_for_dev(&app.env, project_dir);
        let envs = merge_spky_with_user_env(&spky_vars, user_envs);

        // Build the `docker run` argv (owned strings; passed as -e so they reach
        // the container, not the docker process).
        let mut args: Vec<String> = vec![
            "run".into(), "--rm".into(),
            "--name".into(), container.clone(),
            "--network".into(), NETWORK_NAME.into(),
            "--network-alias".into(), name.to_string(),
        ];
        for p in &app.ports {
            args.push("-p".into());
            args.push(p.publish());
        }
        for (k, v) in &envs {
            args.push("-e".into());
            args.push(format!("{}={}", k, v));
        }
        args.push(image.clone());
        for a in &app.args {
            args.push(a.clone());
        }

        println!("{} Starting: docker run {} {}", prefix, image, app.args.join(" "));
        guards.push(spawn_prefixed(
            Command::new("docker").args(&args),
            &prefix,
        ));
    }
    guards
}

fn resolve_workdir(project_dir: &Path, workdir: Option<&str>) -> std::path::PathBuf {
    match workdir {
        Some(dir) => project_dir.join(dir),
        None => project_dir.to_path_buf(),
    }
}

/// Parse a dotenv-style file into key-value pairs.
/// Resolves the path relative to `base_dir`. Skips blank lines and `#` comments.
pub fn load_dotenv_file(path: &Path) -> Vec<(String, String)> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("  Warning: Could not read env-file {:?}: {}", path, e);
            return Vec::new();
        }
    };
    content
        .lines()
        .filter(|l| {
            let trimmed = l.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .filter_map(|l| {
            let (key, value) = l.split_once('=')?;
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

/// Load all vault env vars for dev via the Cloud API directly.
fn load_dev_vault_envs() -> Vec<(String, String)> {
    crate::cloud::load_vault_envs_for_dev()
}

/// Resolve an EnvSource into key-value pairs for dev mode.
fn resolve_env_source(source: &backend::EnvSource, project_dir: &Path) -> Vec<(String, String)> {
    match source {
        backend::EnvSource::Str(s) if s == "vault" => load_dev_vault_envs(),
        backend::EnvSource::Vault(whitelist) => {
            let all = load_dev_vault_envs();
            all.into_iter()
                .filter(|(k, _)| whitelist.iter().any(|w| w == k))
                .collect()
        }
        backend::EnvSource::Str(file_path) => {
            let path = project_dir.join(file_path);
            let envs = load_dotenv_file(&path);
            if !envs.is_empty() {
                println!("  Loaded env-file: {}", path.display());
            }
            envs
        }
        backend::EnvSource::Map(map) => {
            map.iter()
                .map(|(k, v)| {
                    let val = match v {
                        serde_yaml::Value::String(s) => s.clone(),
                        serde_yaml::Value::Number(n) => n.to_string(),
                        serde_yaml::Value::Bool(b) => b.to_string(),
                        other => serde_yaml::to_string(other).unwrap_or_default().trim().to_string(),
                    };
                    (k.clone(), val)
                })
                .collect()
        }
    }
}

/// Resolve an EnvEntry (single source or list) into key-value pairs.
fn resolve_env_entry(entry: &backend::EnvEntry, project_dir: &Path) -> Vec<(String, String)> {
    match entry {
        backend::EnvEntry::Source(source) => resolve_env_source(source, project_dir),
        backend::EnvEntry::List(sources) => {
            let mut merged = std::collections::BTreeMap::new();
            for source in sources {
                for (k, v) in resolve_env_source(source, project_dir) {
                    merged.insert(k, v);
                }
            }
            merged.into_iter().collect()
        }
    }
}

/// Resolve the full EnvConfig for dev mode, merging sources in order.
pub fn resolve_env_for_dev(env: &Option<backend::EnvConfig>, project_dir: &Path) -> Vec<(String, String)> {
    let env = match env {
        Some(e) => e,
        None => return Vec::new(),
    };
    match env {
        backend::EnvConfig::Source(source) => resolve_env_source(source, project_dir),
        backend::EnvConfig::List(sources) => {
            let mut merged = std::collections::BTreeMap::new();
            for source in sources {
                for (k, v) in resolve_env_source(source, project_dir) {
                    merged.insert(k, v);
                }
            }
            merged.into_iter().collect()
        }
        backend::EnvConfig::PerEnvironment { dev, .. } => {
            match dev {
                Some(entry) => resolve_env_entry(entry, project_dir),
                None => Vec::new(),
            }
        }
    }
}

/// Spawn a command with its stdout/stderr prefixed line-by-line.
fn spawn_prefixed(cmd: &mut Command, prefix: &str) -> LogTailGuard {
    let child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    match child {
        Ok(mut c) => {
            if let Some(stdout) = c.stdout.take() {
                let p = prefix.to_string();
                thread::spawn(move || {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines() {
                        match line {
                            Ok(l) => println!("{} {}", p, l),
                            Err(_) => break,
                        }
                    }
                });
            }
            if let Some(stderr) = c.stderr.take() {
                let p = prefix.to_string();
                thread::spawn(move || {
                    let reader = BufReader::new(stderr);
                    for line in reader.lines() {
                        match line {
                            Ok(l) => eprintln!("{} {}", p, l),
                            Err(_) => break,
                        }
                    }
                });
            }
            LogTailGuard(Some(c))
        }
        Err(e) => {
            eprintln!("{} Warning: Could not start process: {}", prefix, e);
            LogTailGuard(None)
        }
    }
}

fn spawn_docker_dev(file: &str, port: Option<&str>, envs: &[(String, String)], cwd: &Path, name: &str, prefix: &str) -> LogTailGuard {
    let tag = format!("sp00ky-dev-{}", name);
    let container_name = format!("sp00ky-dev-app-{}", name);

    // Build the image (blocking, with prefixed output)
    let build_result = Command::new("docker")
        .args(["build", "-f", file, "-t", &tag, "."])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();

    match build_result {
        Ok(output) => {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                println!("{} {}", prefix, line);
            }
            for line in String::from_utf8_lossy(&output.stderr).lines() {
                eprintln!("{} {}", prefix, line);
            }
            if !output.status.success() {
                eprintln!("{} Warning: docker build exited with {}", prefix, output.status);
                return LogTailGuard(None);
            }
        }
        Err(e) => {
            eprintln!("{} Warning: Could not run docker build: {}", prefix, e);
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

    // Pass resolved env vars as -e flags
    for (k, v) in envs {
        args.push("-e".to_string());
        args.push(format!("{}={}", k, v));
    }

    args.push(tag);

    spawn_prefixed(
        Command::new("docker").args(&args).current_dir(cwd),
        prefix,
    )
}

/// Fixed colors for infrastructure services.
const INFRA_COLORS: &[(&str, &str)] = &[
    ("surrealdb",  "\x1b[38;5;208m"), // orange
    ("ssp",        "\x1b[38;5;75m"),  // light blue
    ("scheduler",  "\x1b[38;5;213m"), // pink
];

fn infra_color(label: &str) -> &'static str {
    INFRA_COLORS.iter()
        .find(|(name, _)| *name == label)
        .map(|(_, color)| *color)
        .unwrap_or("\x1b[37m")
}

fn spawn_log_tail(container: &str, label: &str) -> LogTailGuard {
    let prefix = format!("{}[{}]{}", infra_color(label), label, ANSI_RESET);
    spawn_prefixed(
        Command::new("docker").args(["logs", "-f", "--tail", "50", container]),
        &prefix,
    )
}
