use anyhow::{Context, Result};
use openapiv3::OpenAPI;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum SchemaInput {
    Single(String),
    Multiple(Vec<String>),
}

impl SchemaInput {
    pub fn paths(&self) -> Vec<&str> {
        match self {
            SchemaInput::Single(s) => vec![s.as_str()],
            SchemaInput::Multiple(v) => v.iter().map(|s| s.as_str()).collect(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ClientTypeConfig {
    pub format: String,
    pub output: String,
    pub schema: SchemaInput,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SpookyConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// SurrealDB image version (e.g. "v3.0.0"). Separate from spooky service versions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surrealdb: Option<String>,
    /// Spooky service versions — either a string (sets both ssp & scheduler)
    /// or an object `{ ssp: "...", scheduler: "..." }` for individual control.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<VersionConfig>,
    #[serde(default)]
    pub backends: BTreeMap<String, BackendConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buckets: Vec<String>,
    #[serde(default, rename = "clientTypes", skip_serializing_if = "Vec::is_empty")]
    pub client_types: Vec<ClientTypeConfig>,
}

/// Either a plain string (applies to both ssp & scheduler)
/// or an object with individual fields.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum VersionConfig {
    All(String),
    Individual {
        #[serde(default)]
        ssp: Option<String>,
        #[serde(default)]
        scheduler: Option<String>,
    },
}

const DEFAULT_SURREALDB_VERSION: &str = "v3.0.0";
const DEFAULT_SSP_VERSION: &str = "canary";
const DEFAULT_SCHEDULER_VERSION: &str = "canary";

/// Resolved version config with all defaults applied.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ResolvedVersions {
    pub surrealdb: String,
    pub ssp: String,
    pub scheduler: String,
}

impl Default for ResolvedVersions {
    fn default() -> Self {
        Self {
            surrealdb: DEFAULT_SURREALDB_VERSION.to_string(),
            ssp: DEFAULT_SSP_VERSION.to_string(),
            scheduler: DEFAULT_SCHEDULER_VERSION.to_string(),
        }
    }
}

impl ResolvedVersions {
    /// Resolve versions:
    /// - `surrealdb`: from top-level `surrealdb` field, else default
    /// - `version: "tag"` → sets both ssp & scheduler to "tag"
    /// - `version: { ssp: "...", scheduler: "..." }` → individual control, defaults for missing
    pub fn from_config(config: &SpookyConfig) -> Self {
        let surrealdb = config.surrealdb.clone()
            .unwrap_or_else(|| DEFAULT_SURREALDB_VERSION.to_string());

        let (ssp, scheduler) = match &config.version {
            Some(VersionConfig::All(v)) => (v.clone(), v.clone()),
            Some(VersionConfig::Individual { ssp, scheduler }) => (
                ssp.clone().unwrap_or_else(|| DEFAULT_SSP_VERSION.to_string()),
                scheduler.clone().unwrap_or_else(|| DEFAULT_SCHEDULER_VERSION.to_string()),
            ),
            None => (
                DEFAULT_SSP_VERSION.to_string(),
                DEFAULT_SCHEDULER_VERSION.to_string(),
            ),
        };

        Self { surrealdb, ssp, scheduler }
    }

    pub fn surrealdb_image(&self) -> String { format!("surrealdb/surrealdb:{}", self.surrealdb) }
    pub fn ssp_image(&self) -> String { format!("mono424/spooky-ssp:{}", self.ssp) }
    #[allow(dead_code)]
    pub fn scheduler_image(&self) -> String { format!("mono424/spooky-scheduler:{}", self.scheduler) }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct BackendConfig {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub backend_type: Option<String>,
    pub spec: String,
    #[serde(rename = "baseUrl", skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthConfig>,
    pub method: BackendMethod,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AuthConfig {
    #[serde(rename = "type")]
    pub auth_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct BackendMethod {
    #[serde(rename = "type")]
    pub method_type: String,
    pub schema: String,
    pub table: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct BackendRouteArg {
    #[serde(rename = "type")]
    pub arg_type: String,
    pub required: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct BackendRoute {
    pub args: BTreeMap<String, BackendRouteArg>,
}

#[derive(Debug, Serialize, Clone)]
pub struct BackendDefinition {
    pub routes: BTreeMap<String, BackendRoute>,
    pub outbox_table: Option<String>,
}

pub struct BackendProcessor {
    pub schema_appends: String,
    pub backend_definitions: BTreeMap<String, BackendDefinition>,
    pub bucket_schema: String,
}

impl BackendProcessor {
    pub fn new() -> Self {
        Self {
            schema_appends: String::new(),
            backend_definitions: BTreeMap::new(),
            bucket_schema: String::new(),
        }
    }

    pub fn process(&mut self, config_path: &Path) -> Result<()> {
        let config_str = fs::read_to_string(config_path)
            .context(format!("Failed to read spooky config: {:?}", config_path))?;

        let config: SpookyConfig =
            serde_yaml::from_str(&config_str).context("Failed to parse spooky config")?;

        let base_dir = config_path.parent().unwrap_or(Path::new("."));

        for (backend_name, backend_config) in config.backends {
            self.process_backend(&backend_name, &backend_config, base_dir)?;
        }

        for path_str in &config.buckets {
            let bucket_path = base_dir.join(path_str);
            let bucket_content = fs::read_to_string(&bucket_path)
                .context(format!("Failed to read bucket file: {:?}", bucket_path))?;
            self.bucket_schema.push('\n');
            self.bucket_schema.push_str(&bucket_content);
            println!("  + Loaded bucket schema from {:?}", bucket_path);
        }

        Ok(())
    }

    fn process_backend(&mut self, backend_name: &str, backend_config: &BackendConfig, base_dir: &Path) -> Result<()> {
        println!("Processing backend config: {}", backend_name);

        // 1. Append Schema - resolve path relative to spooky.yml
        let schema_path = base_dir.join(&backend_config.method.schema);
        let schema_content = fs::read_to_string(&schema_path)
            .context(format!("Failed to read backend schema: {:?}", schema_path))?;

        self.schema_appends.push('\n');
        self.schema_appends
            .push_str(&format!("-- Backend Schema: {}\n", backend_name));
        self.schema_appends.push_str(&schema_content);
        println!("  + Appended schema from {:?}", schema_path);

        // 2. Parse OpenAPI Spec - resolve path relative to spooky.yml
        let spec_path = base_dir.join(&backend_config.spec);
        let spec_content = fs::read_to_string(&spec_path)
            .context(format!("Failed to read openapi spec: {:?}", spec_path))?;

        let openapi: OpenAPI =
            serde_yaml::from_str(&spec_content).context("Failed to parse openapi spec")?;

        let mut backend_def = BackendDefinition {
            routes: BTreeMap::new(),
            outbox_table: backend_config.method.table.clone(),
        };

        for (path, item) in openapi.paths {
            // We only care about paths that have item content
            let item = match item.as_item() {
                Some(i) => i,
                None => continue,
            };

            // Check for POST method for arguments (as mostly implied by the context of "args")
            // Or should we support all methods? The prompt example shows:
            // backends: { "api": { "/pathA": { args: [...] } } }
            // Let's assume we want to capture arguments from the request body or parameters.

            // For now, let's look at POST operations as they are most likely for RPC-style calls
            if let Some(op) = &item.post {
                let mut args = BTreeMap::new();

                // Extract arguments from Request Body (application/json)
                if let Some(req_body) = &op.request_body {
                    if let Some(req_body_item) = req_body.as_item() {
                        if let Some(content) = req_body_item.content.get("application/json") {
                            if let Some(schema) = &content.schema {
                                if let Some(schema_item) = schema.as_item() {
                                    if let openapiv3::SchemaKind::Type(openapiv3::Type::Object(
                                        obj_type,
                                    )) = &schema_item.schema_kind
                                    {
                                        for (prop_name, prop_schema_ref) in &obj_type.properties {
                                            if let Some(prop_schema_box) = prop_schema_ref.as_item()
                                            {
                                                let prop_schema = &**prop_schema_box;
                                                let arg_type = match &prop_schema.schema_kind {
                                                    openapiv3::SchemaKind::Type(
                                                        openapiv3::Type::String(_),
                                                    ) => "string",
                                                    openapiv3::SchemaKind::Type(
                                                        openapiv3::Type::Number(_),
                                                    ) => "number",
                                                    openapiv3::SchemaKind::Type(
                                                        openapiv3::Type::Integer(_),
                                                    ) => "number",
                                                    openapiv3::SchemaKind::Type(
                                                        openapiv3::Type::Boolean(_),
                                                    ) => "boolean",
                                                    _ => "any", // Fallback
                                                };

                                                let required =
                                                    obj_type.required.contains(prop_name);

                                                args.insert(
                                                    prop_name.clone(),
                                                    BackendRouteArg {
                                                        arg_type: arg_type.to_string(),
                                                        required,
                                                    },
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Also check parameters (query/path) ??
                // The requirements example showed `args: [...]` which usually implies input parameters.
                // Given the context of "spookify" example with "id", it was in the body.
                // I will stick to body properties for now as it matches the spooky RPC style.

                backend_def
                    .routes
                    .insert(path.clone(), BackendRoute { args });
            }
        }

        self.backend_definitions.insert(backend_name.to_string(), backend_def);
        println!("  + Parsed OpenAPI spec from {:?}", spec_path);

        Ok(())
    }
}
