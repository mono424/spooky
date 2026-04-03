use anyhow::{bail, Context, Result};
use inquire::{Confirm, Text};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

use crate::backend::{AuthConfig, BackendConfig, BackendMethod, Sp00kyConfig};

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

    // Step 2: Load or create config
    let mut sp00ky_config: Sp00kyConfig = if config_path.exists() {
        let content = fs::read_to_string(&config_path)
            .context(format!("Failed to read config: {:?}", config_path))?;
        serde_yaml::from_str(&content).context("Failed to parse sp00ky.yml")?
    } else {
        Sp00kyConfig {
            slug: None,
            mode: None,
            surrealdb: None,
            version: None,
            schema: None,
            backends: std::collections::BTreeMap::new(),
            buckets: Vec::new(),
            client_types: Vec::new(),
            dev_app: None,
            frontend: None,
            deployment: None,
            cloud_api: None,
        }
    };

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
            .with_help_message("Used as the key in sp00ky.yml backends section")
            .prompt()?
    };

    // Sanity check: no duplicate
    if sp00ky_config.backends.contains_key(&backend_name) {
        bail!(
            "Backend '{}' already exists in sp00ky.yml",
            backend_name
        );
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
    let auth_config = if let Some(at) = auth_type {
        Some(AuthConfig {
            auth_type: at,
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
                auth_type: "token".to_string(),
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
        fs::create_dir_all(parent)
            .context(format!("Failed to create directory: {:?}", parent))?;
    }

    fs::write(&resolved_schema_output, &surql_content)
        .context(format!("Failed to write schema: {:?}", resolved_schema_output))?;

    // 2. Update sp00ky.yml
    let new_backend = BackendConfig {
        hosting: None,
        backend_type: Some("http".to_string()),
        spec: spec_path_str.clone(),
        base_url: Some(base_url_val.clone()),
        auth: auth_config,
        method: BackendMethod {
            method_type: "outbox".to_string(),
            schema: schema_output_str.clone(),
            table: Some(table_name.clone()),
        },
        dev: None,
        deploy: None,
    };

    sp00ky_config.backends.insert(backend_name.clone(), new_backend);

    let yaml_output = serde_yaml::to_string(&sp00ky_config)
        .context("Failed to serialize config to YAML")?;

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
