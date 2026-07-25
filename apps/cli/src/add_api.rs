use anyhow::{bail, Context, Result};
use inquire::{Confirm, Text};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

use crate::backend::{
    AppConfig, AppType, AuthConfig, AuthType, BackendMethod, MethodType, YAML_SCHEMA_COMMENT,
};

// ── Outbox schema template ──────────────────────────────────────────────────

fn outbox_template(table_name: &str) -> String {
    format!(
        r#"-- ##################################################################
-- API OUTBOX TABLE
-- ##################################################################

DEFINE TABLE {table} SCHEMAFULL
PERMISSIONS
  FOR select, create, update, delete WHERE true;

DEFINE FIELD assigned_to ON TABLE {table} TYPE record
PERMISSIONS
  FOR create, select WHERE true
  FOR update WHERE false;

DEFINE FIELD path ON TABLE {table} TYPE string
PERMISSIONS
  FOR create, select WHERE true
  FOR update WHERE false;

DEFINE FIELD payload ON TABLE {table} TYPE any
PERMISSIONS
  FOR create, select WHERE true
  FOR update WHERE false;

DEFINE FIELD retries ON TABLE {table} TYPE int DEFAULT ALWAYS 0
PERMISSIONS
  FOR create, select WHERE true
  FOR update WHERE false;

DEFINE FIELD max_retries ON TABLE {table} TYPE int DEFAULT ALWAYS 3;

DEFINE FIELD retry_strategy ON TABLE {table} TYPE string DEFAULT ALWAYS "linear"
ASSERT $value IN ["linear", "exponential"]
PERMISSIONS
  FOR create, select WHERE true
  FOR update WHERE false;

DEFINE FIELD status ON TABLE {table} TYPE string DEFAULT ALWAYS "pending"
ASSERT $value IN ["pending", "processing", "success", "failed"]
PERMISSIONS
  FOR create, select WHERE true
  FOR update WHERE false;

DEFINE FIELD errors ON TABLE {table} TYPE array<object> DEFAULT ALWAYS []
PERMISSIONS
  FOR create WHERE true
  FOR select, update WHERE false;

-- The element must be FLEXIBLE or a SCHEMAFULL table rejects the runner's
-- `{{ code, reason }}` entries as unknown fields (`errors[0].code`).
DEFINE FIELD errors[*] ON TABLE {table} TYPE object FLEXIBLE;

DEFINE FIELD updated_at ON TABLE {table} TYPE datetime
DEFAULT ALWAYS time::now()
PERMISSIONS
  FOR create, select WHERE true
  FOR update WHERE false;

DEFINE FIELD created_at ON TABLE {table} TYPE datetime
VALUE time::now()
PERMISSIONS
  FOR create, select WHERE true
  FOR update WHERE false;

-- Platform claim marker: the sp00ky SSP stamps the owning SSP instance
-- (UPDATE ... SET assignee = "ssp-0") when it picks up or recovers a job.
-- Written by the platform as root, never by clients. Deploys also inject it
-- with IF NOT EXISTS as a safety net, but keep it here so the schema file
-- documents the full shape of the table.
DEFINE FIELD assignee ON TABLE {table} TYPE option<string>
PERMISSIONS
  FOR select WHERE true
  FOR create, update WHERE false;

-- The backend's response body on success, stored by the job runner. Readable so
-- you can inspect job output (`spky jobs get`), and it is what a scheduled
-- workflow hands to the steps that depend on this one. Platform-written.
DEFINE FIELD result ON TABLE {table} TYPE any
PERMISSIONS
  FOR select WHERE true
  FOR create, update WHERE false;

-- One-shot delay in ms: the job becomes due at `created_at + delay`
-- (`db.run(..., {{ delay }})`). Recurring work is no longer a field on this
-- table — declare it in sp00ky.yml under `schedules:` and the server-side
-- scheduler creates a fresh row here each cycle.
DEFINE FIELD delay ON TABLE {table} TYPE option<int>
PERMISSIONS
  FOR create, select WHERE true
  FOR update WHERE false;
"#,
        table = table_name
    )
}

// ── Validation ──────────────────────────────────────────────────────────────

fn validate_identifier(name: &str) -> Result<()> {
    let re = Regex::new(r"^[a-z][a-z0-9_]*$")?;
    if !re.is_match(name) {
        bail!(
            "Not a valid SurrealDB identifier: '{}'. Must be lowercase letters, digits, and underscores, starting with a letter.",
            name
        );
    }
    Ok(())
}

// ── Public entry point ──────────────────────────────────────────────────────

pub fn add_api(
    spec: Option<String>,
    name: Option<String>,
    base_url: Option<String>,
    auth_type: Option<String>,
    auth_token: Option<String>,
    table: Option<String>,
    schema_path: Option<String>,
    config: PathBuf,
) -> Result<()> {
    // Step 1: Locate sp00ky.yml
    let config_path = if config.exists() {
        config.clone()
    } else {
        let cwd_config = PathBuf::from("sp00ky.yml");
        if cwd_config.exists() {
            cwd_config
        } else {
            let input = Text::new("Path to sp00ky.yml:")
                .with_default("sp00ky.yml")
                .with_help_message("Will be created if it doesn't exist")
                .prompt()?;
            PathBuf::from(input)
        }
    };

    // Step 2: Load or create config. We work at the raw-YAML level (not the
    // typed `Sp00kyConfig`) so a project that pulls apps in via
    // `apps.<name>.path` — whose reference-only entries would not deserialize
    // into a strict `AppConfig` — round-trips untouched and is never inlined.
    let mut root: serde_yaml::Value = if config_path.exists() {
        let content = fs::read_to_string(&config_path)
            .context(format!("Failed to read config: {:?}", config_path))?;
        serde_yaml::from_str(&content).context("Failed to parse sp00ky.yml")?
    } else {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    };
    if !matches!(root, serde_yaml::Value::Mapping(_)) {
        root = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
    }

    // Step 3: OpenAPI spec path
    let spec_path_str = if let Some(s) = spec {
        s
    } else {
        Text::new("Path to OpenAPI spec:")
            .with_help_message("Relative to sp00ky.yml (e.g. ../api/openapi.yml)")
            .prompt()?
    };

    // Resolve relative to config directory for validation
    let config_dir = config_path.parent().unwrap_or(Path::new("."));
    let resolved_spec = config_dir.join(&spec_path_str);

    // Sanity check: spec file exists
    let spec_content = fs::read_to_string(&resolved_spec)
        .context(format!("Failed to read OpenAPI spec: {:?}", resolved_spec))?;

    // Sanity check: spec parses as YAML
    let openapi: openapiv3::OpenAPI =
        serde_yaml::from_str(&spec_content).context("OpenAPI spec is not valid YAML/JSON")?;

    // Sanity check: spec has at least one path
    if openapi.paths.paths.is_empty() {
        bail!("OpenAPI spec has no endpoints defined");
    }

    // Step 4: Backend name
    let backend_name = if let Some(n) = name {
        n
    } else {
        Text::new("Backend name:")
            .with_default("api")
            .with_help_message("Used as the key in sp00ky.yml apps section")
            .prompt()?
    };

    // Sanity check: no duplicate
    if root
        .get("apps")
        .and_then(|a| a.get(backend_name.as_str()))
        .is_some()
    {
        bail!("App '{}' already exists in sp00ky.yml", backend_name);
    }

    // Step 5: Base URL
    let base_url_val = if let Some(u) = base_url {
        u
    } else {
        Text::new("Base URL:")
            .with_default("http://localhost:3000")
            .with_help_message("The API server base URL")
            .prompt()?
    };

    // Step 6: Auth
    let auth_config = if let Some(_at) = auth_type {
        Some(AuthConfig {
            auth_type: AuthType::Token,
            token: auth_token,
        })
    } else {
        let needs_auth = Confirm::new("Does this API require authentication?")
            .with_default(false)
            .prompt()?;

        if needs_auth {
            let token = Text::new("Auth token:")
                .with_help_message("Bearer token for API authentication")
                .prompt()?;
            Some(AuthConfig {
                auth_type: AuthType::Token,
                token: if token.is_empty() { None } else { Some(token) },
            })
        } else {
            None
        }
    };

    // Step 7: Outbox table name
    let table_name = if let Some(t) = table {
        validate_identifier(&t)?;
        t
    } else {
        let input = Text::new("Outbox table name:")
            .with_default("job")
            .with_help_message("SurrealDB table for the outbox queue")
            .prompt()?;
        validate_identifier(&input)?;
        input
    };

    // Step 8: Schema output path
    let default_schema_path = format!("./src/outbox/{}.surql", backend_name);
    let schema_output_str = if let Some(sp) = schema_path {
        sp
    } else {
        Text::new("Schema output path:")
            .with_default(&default_schema_path)
            .with_help_message("Where to write the outbox .surql file (relative to sp00ky.yml)")
            .prompt()?
    };

    let resolved_schema_output = config_dir.join(&schema_output_str);

    // Sanity check: schema file doesn't already exist (or confirm overwrite)
    if resolved_schema_output.exists() {
        let overwrite = Confirm::new(&format!(
            "{} already exists. Overwrite?",
            resolved_schema_output.display()
        ))
        .with_default(false)
        .prompt()?;

        if !overwrite {
            println!("  Aborted.");
            return Ok(());
        }
    }

    // ── Actions ─────────────────────────────────────────────────────────────

    // 1. Generate and write outbox schema
    let surql_content = outbox_template(&table_name);

    if let Some(parent) = resolved_schema_output.parent() {
        fs::create_dir_all(parent).context(format!("Failed to create directory: {:?}", parent))?;
    }

    fs::write(&resolved_schema_output, &surql_content).context(format!(
        "Failed to write schema: {:?}",
        resolved_schema_output
    ))?;

    // 2. Update sp00ky.yml
    let new_app = AppConfig {
        app_type: AppType::Backend,
        scope: Default::default(),
        hosting: None,
        spec: Some(spec_path_str.clone()),
        base_url: Some(base_url_val.clone()),
        auth: auth_config,
        method: Some(BackendMethod {
            method_type: MethodType::Outbox,
            schema: schema_output_str.clone(),
            table: Some(table_name.clone()),
        }),
        image: None,
        ports: Vec::new(),
        args: Vec::new(),
        volumes: Vec::new(),
        workdir: None,
        depends_on: Vec::new(),
        healthcheck: None,
        dev: None,
        deploy: None,
        env: None,
    };

    // Insert the new app as a YAML node under `apps`, creating the map if the
    // file had none. Existing entries (including `path:` references) are left
    // byte-for-byte as authored.
    let new_app_value =
        serde_yaml::to_value(&new_app).context("Failed to serialize new app to YAML")?;
    if let serde_yaml::Value::Mapping(root_map) = &mut root {
        let apps_key = serde_yaml::Value::String("apps".to_string());
        let apps = root_map
            .entry(apps_key)
            .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
        if let serde_yaml::Value::Mapping(apps_map) = apps {
            apps_map.insert(
                serde_yaml::Value::String(backend_name.clone()),
                new_app_value,
            );
        } else {
            bail!("`apps` in sp00ky.yml is not a mapping");
        }
    }

    let yaml_output =
        serde_yaml::to_string(&root).context("Failed to serialize config to YAML")?;
    let yaml_output = format!("{}\n{}", YAML_SCHEMA_COMMENT, yaml_output);

    fs::write(&config_path, &yaml_output)
        .context(format!("Failed to write config: {:?}", config_path))?;

    // 3. Print summary
    println!();
    println!("  API Backend Added");
    println!("  ─────────────────────────────────");
    println!("  Name:        {}", backend_name);
    println!("  Spec:        {}", spec_path_str);
    println!("  Base URL:    {}", base_url_val);
    println!("  Table:       {}", table_name);
    println!("  Schema:      {}", schema_output_str);
    println!("  Config:      {} (updated)", config_path.display());
    println!();
    println!("  Run `sp00ky` to regenerate types with the new backend.");
    println!();

    Ok(())
}
