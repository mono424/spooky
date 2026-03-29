use anyhow::{bail, Context, Result};
use openapiv3::OpenAPI;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Whether a service is hosted on Sp00ky Cloud or externally.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum HostingMode {
    Cloud,
    External,
}

impl Default for HostingMode {
    fn default() -> Self {
        Self::Cloud
    }
}

pub const DEFAULT_SCHEMA_PATH: &str = "src/schema.surql";
pub const DEFAULT_MIGRATIONS_DIR: &str = "migrations";
pub const DEFAULT_BUCKETS_DIR: &str = "src/buckets";
pub const DEFAULT_CONFIG_PATH: &str = "sp00ky.yml";

/// SurrealDB config: either a plain version string (backwards compat)
/// or an object with version, namespace, and database.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum SurrealDbConfig {
    /// Just the image version, e.g. "v3.0.0"
    Version(String),
    /// Full config with optional fields
    Full {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        namespace: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<String>,
        /// "cloud" (default) or "external"
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hosting: Option<HostingMode>,
        /// Required when hosting is "external" — the SurrealDB endpoint URL
        #[serde(default, skip_serializing_if = "Option::is_none")]
        endpoint: Option<String>,
    },
}

/// Resolved SurrealDB config with all defaults applied.
#[derive(Debug, Clone)]
pub struct ResolvedSurrealDb {
    pub version: String,
    pub namespace: String,
    pub database: String,
    pub hosting: HostingMode,
    /// Only `Some` when `hosting == External`.
    pub endpoint: Option<String>,
}

impl ResolvedSurrealDb {
    pub fn from_config(config: &Option<SurrealDbConfig>) -> Self {
        match config {
            Some(SurrealDbConfig::Version(v)) => Self {
                version: v.clone(),
                namespace: "main".to_string(),
                database: "main".to_string(),
                hosting: HostingMode::Cloud,
                endpoint: None,
            },
            Some(SurrealDbConfig::Full { version, namespace, database, hosting, endpoint }) => Self {
                version: version.clone().unwrap_or_else(|| DEFAULT_SURREALDB_VERSION.to_string()),
                namespace: namespace.clone().unwrap_or_else(|| "main".to_string()),
                database: database.clone().unwrap_or_else(|| "main".to_string()),
                hosting: hosting.clone().unwrap_or_default(),
                endpoint: endpoint.clone(),
            },
            None => Self {
                version: DEFAULT_SURREALDB_VERSION.to_string(),
                namespace: "main".to_string(),
                database: "main".to_string(),
                hosting: HostingMode::Cloud,
                endpoint: None,
            },
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.hosting == HostingMode::External && self.endpoint.is_none() {
            bail!("SurrealDB hosting is 'external' but no endpoint URL was provided");
        }
        Ok(())
    }
}

/// Schema config: either a directory string (sub-paths derived by convention)
/// or an object with explicit overrides.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum SchemaConfig {
    Dir(String),
    Explicit {
        schema: Option<String>,
        migrations: Option<String>,
        #[serde(rename = "bucketsDir")]
        buckets_dir: Option<String>,
    },
}

/// Resolved schema paths with all defaults applied.
#[derive(Debug, Clone)]
pub struct ResolvedSchema {
    pub schema: PathBuf,
    pub migrations: PathBuf,
    pub buckets_dir: PathBuf,
}

impl ResolvedSchema {
    pub fn from_config(config: &Option<SchemaConfig>) -> Self {
        match config {
            Some(SchemaConfig::Dir(dir)) => Self {
                schema: PathBuf::from(dir).join(DEFAULT_SCHEMA_PATH),
                migrations: PathBuf::from(dir).join(DEFAULT_MIGRATIONS_DIR),
                buckets_dir: PathBuf::from(dir).join(DEFAULT_BUCKETS_DIR),
            },
            Some(SchemaConfig::Explicit { schema, migrations, buckets_dir }) => Self {
                schema: PathBuf::from(schema.as_deref().unwrap_or(DEFAULT_SCHEMA_PATH)),
                migrations: PathBuf::from(migrations.as_deref().unwrap_or(DEFAULT_MIGRATIONS_DIR)),
                buckets_dir: PathBuf::from(buckets_dir.as_deref().unwrap_or(DEFAULT_BUCKETS_DIR)),
            },
            None => Self {
                schema: PathBuf::from(DEFAULT_SCHEMA_PATH),
                migrations: PathBuf::from(DEFAULT_MIGRATIONS_DIR),
                buckets_dir: PathBuf::from(DEFAULT_BUCKETS_DIR),
            },
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ClientTypeConfig {
    pub format: String,
    pub output: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Sp00kyConfig {
    /// Cloud project slug (used by `sp00ky cloud` commands)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// SurrealDB config: version string or object with version/namespace/database.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surrealdb: Option<SurrealDbConfig>,
    /// Sp00ky service versions — either a string (sets both ssp & scheduler)
    /// or an object `{ ssp: "...", scheduler: "..." }` for individual control.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<VersionConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<SchemaConfig>,
    #[serde(default)]
    pub backends: BTreeMap<String, BackendConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buckets: Vec<String>,
    #[serde(default, rename = "clientTypes", skip_serializing_if = "Vec::is_empty")]
    pub client_types: Vec<ClientTypeConfig>,
    #[serde(default, rename = "devApp", skip_serializing_if = "Option::is_none")]
    pub dev_app: Option<String>,
    /// Frontend deployment configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontend: Option<FrontendDeployConfig>,
    /// Override the Sp00ky Cloud API endpoint (e.g. for staging).
    #[serde(default, rename = "cloudApi", skip_serializing_if = "Option::is_none")]
    pub cloud_api: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct FrontendDeployConfig {
    /// Dockerfile path (relative to sp00ky.yml)
    pub dockerfile: String,
    /// Build context directory (relative to sp00ky.yml, defaults to project root)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Port the frontend listens on inside the container (default: 3000)
    #[serde(default = "default_frontend_port")]
    pub port: u16,
    /// Resource allocation
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<BackendDeployResources>,
    /// Environment variables
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
}

fn default_frontend_port() -> u16 {
    3000
}

impl Sp00kyConfig {
    pub fn resolved_schema(&self) -> ResolvedSchema {
        ResolvedSchema::from_config(&self.schema)
    }

    pub fn resolved_surrealdb(&self) -> ResolvedSurrealDb {
        ResolvedSurrealDb::from_config(&self.surrealdb)
    }

    /// Validate hosting configuration for SurrealDB and all backends.
    pub fn validate(&self) -> Result<()> {
        self.resolved_surrealdb().validate()?;
        for (name, backend) in &self.backends {
            backend.validate(name)?;
        }
        Ok(())
    }
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
    pub fn from_config(config: &Sp00kyConfig) -> Self {
        let surrealdb = config.resolved_surrealdb().version;

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

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum BackendDevConfig {
    /// Raw shell command string, e.g. "node server.js"
    Command(String),
    /// Typed object form with type discriminator
    Typed(BackendDevTypedConfig),
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(tag = "type")]
pub enum BackendDevTypedConfig {
    #[serde(rename = "npm")]
    Npm {
        script: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workdir: Option<String>,
        #[serde(default, rename = "env-file", skip_serializing_if = "Option::is_none")]
        env_file: Option<String>,
    },
    #[serde(rename = "docker")]
    Docker {
        file: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workdir: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        env: Vec<String>,
        #[serde(default, rename = "env-file", skip_serializing_if = "Option::is_none")]
        env_file: Option<String>,
    },
    #[serde(rename = "uv")]
    Uv {
        script: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workdir: Option<String>,
        #[serde(default, rename = "env-file", skip_serializing_if = "Option::is_none")]
        env_file: Option<String>,
    },
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct BackendDeployConfig {
    /// Resource allocation for the backend VM
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<BackendDeployResources>,
    /// Expose publicly via {slug}-{name}.fn.spky.cloud
    #[serde(default)]
    pub expose: bool,
    /// Port the service listens on inside the container (default: 8080)
    #[serde(default = "default_deploy_port")]
    pub port: u16,
    /// Environment variables passed to the VM
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
    /// Override Dockerfile path (defaults to dev.docker.file if available)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dockerfile: Option<String>,
    /// Build context directory (relative to sp00ky.yml, defaults to project root)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

fn default_deploy_port() -> u16 {
    8080
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct BackendDeployResources {
    /// Number of vCPUs (default: 1)
    #[serde(default = "default_vcpus")]
    pub vcpus: u32,
    /// Memory in MB (default: 512)
    #[serde(default = "default_memory")]
    pub memory: u32,
    /// Disk in GB (default: 5)
    #[serde(default = "default_disk")]
    pub disk: u32,
}

fn default_vcpus() -> u32 { 1 }
fn default_memory() -> u32 { 512 }
fn default_disk() -> u32 { 5 }

#[derive(Debug, Deserialize, Serialize)]
pub struct BackendConfig {
    /// "cloud" (default) or "external" — whether this backend is deployed to
    /// Sp00ky Cloud or self-hosted at `baseUrl`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hosting: Option<HostingMode>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub backend_type: Option<String>,
    pub spec: String,
    #[serde(rename = "baseUrl", skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthConfig>,
    pub method: BackendMethod,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev: Option<BackendDevConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deploy: Option<BackendDeployConfig>,
}

impl BackendConfig {
    pub fn resolved_hosting(&self) -> HostingMode {
        self.hosting.clone().unwrap_or_default()
    }

    pub fn validate(&self, name: &str) -> Result<()> {
        if self.resolved_hosting() == HostingMode::External && self.base_url.is_none() {
            bail!("Backend '{}' has hosting 'external' but no baseUrl was provided", name);
        }
        Ok(())
    }
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

/// Load and parse a Sp00kyConfig from the given path.
/// Returns a default config if the file doesn't exist or can't be parsed.
pub fn load_config(path: &Path) -> Sp00kyConfig {
    if !path.exists() {
        return default_config();
    }

    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return default_config(),
    };

    match serde_yaml::from_str(&content) {
        Ok(c) => c,
        Err(_) => default_config(),
    }
}

fn default_config() -> Sp00kyConfig {
    Sp00kyConfig {
        slug: None,
        mode: Some("singlenode".to_string()),
        surrealdb: None,
        version: None,
        schema: None,
        backends: Default::default(),
        buckets: Default::default(),
        client_types: Default::default(),
        dev_app: None,
        frontend: None,
        cloud_api: None,
    }
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
            .context(format!("Failed to read sp00ky config: {:?}", config_path))?;

        let config: Sp00kyConfig =
            serde_yaml::from_str(&config_str).context("Failed to parse sp00ky config")?;

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

        // 1. Append Schema - resolve path relative to sp00ky.yml
        let schema_path = base_dir.join(&backend_config.method.schema);
        let schema_content = fs::read_to_string(&schema_path)
            .context(format!("Failed to read backend schema: {:?}", schema_path))?;

        self.schema_appends.push('\n');
        self.schema_appends
            .push_str(&format!("-- Backend Schema: {}\n", backend_name));
        self.schema_appends.push_str(&schema_content);
        println!("  + Appended schema from {:?}", schema_path);

        // 2. Parse OpenAPI Spec - resolve path relative to sp00ky.yml
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
                // I will stick to body properties for now as it matches the sp00ky RPC style.

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
