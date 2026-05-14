use anyhow::{bail, Context, Result};
use openapiv3::OpenAPI;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

// ── App Type ────────────────────────────────────────────────────────────────

/// Discriminator for app type — must be specified explicitly.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AppType {
    Backend,
    Frontend,
}

/// Deployment mode.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DeployMode {
    Singlenode,
    Cluster,
    Surrealism,
}

impl Default for DeployMode {
    fn default() -> Self {
        Self::Singlenode
    }
}

impl std::fmt::Display for DeployMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeployMode::Singlenode => write!(f, "singlenode"),
            DeployMode::Cluster => write!(f, "cluster"),
            DeployMode::Surrealism => write!(f, "surrealism"),
        }
    }
}

/// Authentication type for backend services.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AuthType {
    Token,
}

/// Backend trigger method type.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MethodType {
    Outbox,
}

/// Client type generation format.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ClientFormat {
    Typescript,
    Dart,
}

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
pub const YAML_SCHEMA_COMMENT: &str = "# yaml-language-server: $schema=https://sp00ky.cloud/schema/sp00ky.schema.json";

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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        username: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<String>,
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
    pub username: String,
    pub password: String,
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
                username: "root".to_string(),
                password: "root".to_string(),
            },
            Some(SurrealDbConfig::Full { version, namespace, database, hosting, endpoint, username, password }) => Self {
                version: version.clone().unwrap_or_else(|| DEFAULT_SURREALDB_VERSION.to_string()),
                namespace: namespace.clone().unwrap_or_else(|| "main".to_string()),
                database: database.clone().unwrap_or_else(|| "main".to_string()),
                hosting: hosting.clone().unwrap_or_default(),
                endpoint: endpoint.clone(),
                username: username.clone().unwrap_or_else(|| "root".to_string()),
                password: password.clone().unwrap_or_else(|| "root".to_string()),
            },
            None => Self {
                version: DEFAULT_SURREALDB_VERSION.to_string(),
                namespace: "main".to_string(),
                database: "main".to_string(),
                hosting: HostingMode::Cloud,
                endpoint: None,
                username: "root".to_string(),
                password: "root".to_string(),
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
    pub format: ClientFormat,
    pub output: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Sp00kyConfig {
    /// Cloud project slug (used by `sp00ky cloud` commands)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<DeployMode>,
    /// SurrealDB config: version string or object with version/namespace/database.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surrealdb: Option<SurrealDbConfig>,
    /// Sp00ky service versions — either a string (sets both ssp & scheduler)
    /// or an object `{ ssp: "...", scheduler: "..." }` for individual control.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<VersionConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<SchemaConfig>,
    /// Application definitions (backends and frontends). Each key is the app name.
    #[serde(default)]
    pub apps: BTreeMap<String, AppConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buckets: Vec<String>,
    #[serde(default, rename = "clientTypes", skip_serializing_if = "Vec::is_empty")]
    pub client_types: Vec<ClientTypeConfig>,
    /// Deployment configuration (SSP count, scaling options)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment: Option<DeploymentConfig>,
    /// Override the Sp00ky Cloud API endpoint (e.g. for staging).
    #[serde(default, rename = "cloudApi", skip_serializing_if = "Option::is_none")]
    pub cloud_api: Option<String>,
    /// Migration engine to use: "legacy" (default) or "surrealkit".
    #[serde(default, rename = "migrationEngine", skip_serializing_if = "Option::is_none")]
    pub migration_engine: Option<String>,
    /// SurrealKit-specific configuration (only used when migrationEngine = "surrealkit").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surrealkit: Option<SurrealKitConfig>,
    /// `RUST_LOG` directive applied to the scheduler and SSP containers.
    /// Either a plain string (`trace`, `info`, `info,ssp=debug`, …) or a
    /// per-environment map `{ dev, cloud }`. Unset → defaults to `info`.
    #[serde(default, rename = "logLevel", skip_serializing_if = "Option::is_none")]
    pub log_level: Option<LogLevelConfig>,
    /// Storage mode for the internal `_00_query` / `_00_list_ref` ref tables.
    /// `single` (legacy): one shared table per kind. `dedicated` (default):
    /// per-user tables `_00_{query,list_ref}_user_<id>`, created lazily by
    /// the SSP on first registration. Dedicated mode works around a
    /// SurrealDB v3 LIVE-permission gap where cross-session INSERTs to a
    /// permission-gated table never fire LIVE notifications for the
    /// non-inserting subscriber.
    #[serde(default, rename = "refMode", skip_serializing_if = "Option::is_none")]
    pub ref_mode: Option<RefMode>,
}

// `RefMode` lives in `ssp-protocol` so the CLI, the SSP server, and any
// other crate that needs to derive table names share a single source of
// truth. Re-export here so existing import paths in the CLI keep working.
pub use ssp_protocol::RefMode;

impl Sp00kyConfig {
    /// Resolved ref mode, falling back to the default when unset in YAML.
    pub fn resolved_ref_mode(&self) -> RefMode {
        self.ref_mode.unwrap_or_default()
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SurrealKitConfig {
    /// Path to the surrealkit binary. Defaults to "surrealkit" (found via PATH).
    pub binary: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DeploymentConfig {
    /// Number of SSP instances to provision (overrides plan default)
    #[serde(default, rename = "sspCount", skip_serializing_if = "Option::is_none")]
    pub ssp_count: Option<u32>,
    /// Backup configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<BackupConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct BackupConfig {
    /// Enable automated backups
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Cron schedule (e.g., "0 2 * * *" for 2am daily)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
    /// Number of backups to retain
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention: Option<u32>,
    /// External S3-compatible bucket URL (skip MinIO if set)
    #[serde(default, rename = "bucketUrl", skip_serializing_if = "Option::is_none")]
    pub bucket_url: Option<String>,
    /// Path to env file with BACKUP_ACCESS_KEY and BACKUP_SECRET_KEY
    #[serde(default, rename = "credentialsEnvFile", skip_serializing_if = "Option::is_none")]
    pub credentials_env_file: Option<String>,
}

// ── Unified Env Config ──────────────────────────────────────────────────────

/// A single environment variable source: "vault", a dotenv file path, or an inline map.
#[derive(Debug, Clone)]
pub enum EnvSource {
    /// "vault" (all vars) or a dotenv file path
    Str(String),
    /// Inline key-value map, e.g. `{ DB_URL: "localhost", PORT: 3000 }`
    Map(BTreeMap<String, serde_yaml::Value>),
    /// Vault with a whitelist of variable names, e.g. `{ vault: [DB_URL, API_KEY] }`
    Vault(Vec<String>),
}

impl Serialize for EnvSource {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            EnvSource::Str(s) => serializer.serialize_str(s),
            EnvSource::Map(m) => m.serialize(serializer),
            EnvSource::Vault(keys) => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("vault", keys)?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for EnvSource {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = serde_yaml::Value::deserialize(deserializer)?;
        match &value {
            serde_yaml::Value::String(s) => Ok(EnvSource::Str(s.clone())),
            serde_yaml::Value::Mapping(m) => {
                // Check for vault whitelist: { vault: [KEY1, KEY2, ...] }
                let vault_key = serde_yaml::Value::String("vault".into());
                if m.len() == 1 {
                    if let Some(val) = m.get(&vault_key) {
                        if let serde_yaml::Value::Sequence(seq) = val {
                            let keys: Vec<String> = seq.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect();
                            return Ok(EnvSource::Vault(keys));
                        }
                    }
                }
                // Otherwise it's an inline key-value map
                let map = m.iter()
                    .filter_map(|(k, v)| k.as_str().map(|s| (s.to_string(), v.clone())))
                    .collect();
                Ok(EnvSource::Map(map))
            }
            _ => Err(serde::de::Error::custom("env source must be a string or a map")),
        }
    }
}

/// An env entry used inside `PerEnvironment`: a single source or a list of sources.
#[derive(Debug, Clone)]
pub enum EnvEntry {
    Source(EnvSource),
    List(Vec<EnvSource>),
}

impl Serialize for EnvEntry {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            EnvEntry::Source(s) => s.serialize(serializer),
            EnvEntry::List(l) => l.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for EnvEntry {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = serde_yaml::Value::deserialize(deserializer)?;
        match &value {
            serde_yaml::Value::Sequence(_) => {
                let sources: Vec<EnvSource> = serde_yaml::from_value(value)
                    .map_err(serde::de::Error::custom)?;
                Ok(EnvEntry::List(sources))
            }
            _ => {
                let source: EnvSource = serde_yaml::from_value(value)
                    .map_err(serde::de::Error::custom)?;
                Ok(EnvEntry::Source(source))
            }
        }
    }
}

/// Environment variable configuration.
///
/// Supports:
/// - `"vault"` or `"path/to/file"` — single string source
/// - `{ KEY: "val" }` — inline key-value map
/// - `{ dev: <entry>, cloud: <entry> }` — per-environment split
/// - `["vault", ".env", { KEY: "val" }]` — array of sources, merged in order
#[derive(Debug, Clone)]
pub enum EnvConfig {
    Source(EnvSource),
    PerEnvironment {
        dev: Option<EnvEntry>,
        cloud: Option<EnvEntry>,
    },
    List(Vec<EnvSource>),
}

impl Serialize for EnvConfig {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            EnvConfig::Source(s) => s.serialize(serializer),
            EnvConfig::PerEnvironment { dev, cloud } => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(None)?;
                if let Some(d) = dev { map.serialize_entry("dev", d)?; }
                if let Some(c) = cloud { map.serialize_entry("cloud", c)?; }
                map.end()
            }
            EnvConfig::List(l) => l.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for EnvConfig {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = serde_yaml::Value::deserialize(deserializer)?;
        match &value {
            serde_yaml::Value::String(s) => Ok(EnvConfig::Source(EnvSource::Str(s.clone()))),
            serde_yaml::Value::Sequence(_) => {
                let sources: Vec<EnvSource> = serde_yaml::from_value(value)
                    .map_err(serde::de::Error::custom)?;
                Ok(EnvConfig::List(sources))
            }
            serde_yaml::Value::Mapping(m) => {
                // If the object has ONLY "dev" and/or "cloud" keys → PerEnvironment
                let keys: Vec<&str> = m.keys().filter_map(|k| k.as_str()).collect();
                let is_per_env = !keys.is_empty()
                    && keys.iter().all(|k| *k == "dev" || *k == "cloud");

                if is_per_env {
                    let dev_key = serde_yaml::Value::String("dev".into());
                    let cloud_key = serde_yaml::Value::String("cloud".into());
                    let dev = m.get(&dev_key)
                        .map(|v| serde_yaml::from_value::<EnvEntry>(v.clone()))
                        .transpose()
                        .map_err(serde::de::Error::custom)?;
                    let cloud = m.get(&cloud_key)
                        .map(|v| serde_yaml::from_value::<EnvEntry>(v.clone()))
                        .transpose()
                        .map_err(serde::de::Error::custom)?;
                    Ok(EnvConfig::PerEnvironment { dev, cloud })
                } else {
                    // Delegate to EnvSource which handles vault whitelist + inline maps
                    let source: EnvSource = serde_yaml::from_value(value)
                        .map_err(serde::de::Error::custom)?;
                    Ok(EnvConfig::Source(source))
                }
            }
            _ => Err(serde::de::Error::custom("env config must be a string, map, or array")),
        }
    }
}

impl Sp00kyConfig {
    pub fn resolved_schema(&self) -> ResolvedSchema {
        ResolvedSchema::from_config(&self.schema)
    }

    pub fn resolved_surrealdb(&self) -> ResolvedSurrealDb {
        ResolvedSurrealDb::from_config(&self.surrealdb)
    }

    /// Iterate over backend apps only.
    pub fn backends(&self) -> impl Iterator<Item = (&str, &AppConfig)> {
        self.apps.iter()
            .filter(|(_, app)| app.app_type == AppType::Backend)
            .map(|(name, app)| (name.as_str(), app))
    }

    /// Return the first frontend app, if any.
    pub fn frontend(&self) -> Option<(&str, &AppConfig)> {
        self.apps.iter()
            .find(|(_, app)| app.app_type == AppType::Frontend)
            .map(|(name, app)| (name.as_str(), app))
    }

    /// Resolve the surrealkit binary path (if migration engine is "surrealkit").
    pub fn resolved_surrealkit_binary(&self) -> Option<String> {
        if self.migration_engine.as_deref() == Some("surrealkit") {
            Some(
                self.surrealkit
                    .as_ref()
                    .and_then(|c| c.binary.clone())
                    .unwrap_or_else(|| "surrealkit".to_string()),
            )
        } else {
            None
        }
    }

    /// Validate hosting configuration for SurrealDB and all apps.
    pub fn validate(&self) -> Result<()> {
        self.resolved_surrealdb().validate()?;
        for (name, app) in &self.apps {
            app.validate(name)?;
        }
        // logLevel: walk every directive string and confirm it parses.
        if let Some(cfg) = &self.log_level {
            match cfg {
                LogLevelConfig::Single(s) => validate_rust_log(s)?,
                LogLevelConfig::PerEnvironment { dev, cloud } => {
                    if let Some(s) = dev { validate_rust_log(s)?; }
                    if let Some(s) = cloud { validate_rust_log(s)?; }
                }
            }
        }
        Ok(())
    }

    /// Resolved `RUST_LOG` value for the given environment. Falls back to
    /// `info` when `logLevel` is unset or has no entry for the requested env,
    /// preserving today's behavior for projects that don't opt in.
    pub fn resolved_log_level(&self, env: DeployEnv) -> String {
        self.log_level
            .as_ref()
            .and_then(|c| c.resolved(env))
            .unwrap_or_else(|| "info".to_string())
    }
}

/// Per-component pin: either a Docker tag, or a path to a host binary.
/// A bare string parses as `Tag`, the object form `{ path: "..." }` parses
/// as `Path`. Path-mode opts that one component into "host process" launch;
/// see `RuntimeSource` and `dev::run_direct_mode`.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged, deny_unknown_fields)]
pub enum ComponentVersion {
    /// Bare string => Docker tag (e.g. "canary", "v0.1.0").
    Tag(String),
    /// `{ path: "../../target/debug/ssp-server" }` => host process.
    Path {
        path: PathBuf,
    },
}

/// Inner shape: a string (applies to both ssp & scheduler) or `{ssp, scheduler}`.
/// Used both as the flat `version` value and as the per-env entry inside `VersionConfig::PerEnvironment`.
///
/// `All` is intentionally tag-only — a single value applied to both components
/// can't be a path, since the SSP and scheduler are different binaries. Use
/// `Individual` for path-mode.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged, deny_unknown_fields)]
pub enum VersionSpec {
    All(String),
    Individual {
        #[serde(default)]
        ssp: Option<ComponentVersion>,
        #[serde(default)]
        scheduler: Option<ComponentVersion>,
    },
}

/// Either a single spec applied everywhere, or one spec per environment.
/// Custom (de)serialize disambiguates `{ssp, scheduler}` from `{dev, cloud}` by key inspection,
/// mirroring `EnvConfig`.
#[derive(Debug, Clone)]
pub enum VersionConfig {
    Single(VersionSpec),
    PerEnvironment {
        dev: Option<VersionSpec>,
        cloud: Option<VersionSpec>,
    },
}

impl Serialize for VersionConfig {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            VersionConfig::Single(s) => s.serialize(serializer),
            VersionConfig::PerEnvironment { dev, cloud } => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(None)?;
                if let Some(d) = dev { map.serialize_entry("dev", d)?; }
                if let Some(c) = cloud { map.serialize_entry("cloud", c)?; }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for VersionConfig {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = serde_yaml::Value::deserialize(deserializer)?;
        match &value {
            serde_yaml::Value::String(_) => {
                let spec: VersionSpec = serde_yaml::from_value(value)
                    .map_err(serde::de::Error::custom)?;
                Ok(VersionConfig::Single(spec))
            }
            serde_yaml::Value::Mapping(m) => {
                // If the object has ONLY "dev" and/or "cloud" keys → PerEnvironment.
                let keys: Vec<&str> = m.keys().filter_map(|k| k.as_str()).collect();
                let is_per_env = !keys.is_empty()
                    && keys.iter().all(|k| *k == "dev" || *k == "cloud");

                if is_per_env {
                    let dev_key = serde_yaml::Value::String("dev".into());
                    let cloud_key = serde_yaml::Value::String("cloud".into());
                    let dev = m.get(&dev_key)
                        .map(|v| serde_yaml::from_value::<VersionSpec>(v.clone()))
                        .transpose()
                        .map_err(serde::de::Error::custom)?;
                    let cloud = m.get(&cloud_key)
                        .map(|v| serde_yaml::from_value::<VersionSpec>(v.clone()))
                        .transpose()
                        .map_err(serde::de::Error::custom)?;
                    Ok(VersionConfig::PerEnvironment { dev, cloud })
                } else {
                    // Empty map or other keys → fall through to VersionSpec::Individual,
                    // whose `deny_unknown_fields` will reject typos.
                    let spec: VersionSpec = serde_yaml::from_value(value)
                        .map_err(serde::de::Error::custom)?;
                    Ok(VersionConfig::Single(spec))
                }
            }
            _ => Err(serde::de::Error::custom("version must be a string or map")),
        }
    }
}

/// `RUST_LOG` directive shape — same dual-form idiom as `VersionConfig`.
/// A plain string applies in every environment; a `{ dev, cloud }` map sets
/// per-environment levels. `LogLevelConfig::resolved(env)` collapses both
/// shapes to an `Option<String>`.
#[derive(Debug, Clone)]
pub enum LogLevelConfig {
    Single(String),
    PerEnvironment {
        dev: Option<String>,
        cloud: Option<String>,
    },
}

impl LogLevelConfig {
    /// Resolve the level for a given environment. Returns `None` when the
    /// per-env map has no entry for `env` so the caller can apply a default.
    pub fn resolved(&self, env: DeployEnv) -> Option<String> {
        match self {
            LogLevelConfig::Single(s) => Some(s.clone()),
            LogLevelConfig::PerEnvironment { dev, cloud } => match env {
                DeployEnv::Dev => dev.clone(),
                DeployEnv::Cloud => cloud.clone(),
            },
        }
    }
}

impl Serialize for LogLevelConfig {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            LogLevelConfig::Single(s) => serializer.serialize_str(s),
            LogLevelConfig::PerEnvironment { dev, cloud } => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(None)?;
                if let Some(d) = dev { map.serialize_entry("dev", d)?; }
                if let Some(c) = cloud { map.serialize_entry("cloud", c)?; }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for LogLevelConfig {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = serde_yaml::Value::deserialize(deserializer)?;
        match &value {
            serde_yaml::Value::String(s) => Ok(LogLevelConfig::Single(s.clone())),
            serde_yaml::Value::Mapping(m) => {
                let keys: Vec<&str> = m.keys().filter_map(|k| k.as_str()).collect();
                let known = keys.iter().all(|k| *k == "dev" || *k == "cloud");
                if !known || keys.is_empty() {
                    return Err(serde::de::Error::custom(
                        "logLevel map must contain only `dev` and/or `cloud` keys",
                    ));
                }
                let dev = m.get(serde_yaml::Value::String("dev".into()))
                    .map(|v| v.as_str().map(String::from)
                        .ok_or_else(|| serde::de::Error::custom("logLevel.dev must be a string")))
                    .transpose()?;
                let cloud = m.get(serde_yaml::Value::String("cloud".into()))
                    .map(|v| v.as_str().map(String::from)
                        .ok_or_else(|| serde::de::Error::custom("logLevel.cloud must be a string")))
                    .transpose()?;
                Ok(LogLevelConfig::PerEnvironment { dev, cloud })
            }
            _ => Err(serde::de::Error::custom("logLevel must be a string or { dev, cloud } map")),
        }
    }
}

/// Validate a `RUST_LOG` directive string. Accepts either a bare level
/// (`trace|debug|info|warn|error|off`) or a comma-separated list of
/// `target=level` directives matching the `tracing-subscriber` grammar.
fn validate_rust_log(s: &str) -> Result<()> {
    if s.trim().is_empty() {
        anyhow::bail!("logLevel value cannot be empty");
    }
    let valid_level = |lv: &str| {
        matches!(lv, "trace" | "debug" | "info" | "warn" | "error" | "off")
    };
    for token in s.split(',') {
        let token = token.trim();
        if token.is_empty() {
            anyhow::bail!("logLevel `{}` has empty directive", s);
        }
        if let Some((_target, level)) = token.split_once('=') {
            if !valid_level(level.trim()) {
                anyhow::bail!(
                    "logLevel `{}` — directive `{}` has invalid level (use trace|debug|info|warn|error|off)",
                    s, token
                );
            }
        } else if !valid_level(token) {
            anyhow::bail!(
                "logLevel `{}` — `{}` is not a valid level (use trace|debug|info|warn|error|off, or `target=level`)",
                s, token
            );
        }
    }
    Ok(())
}

/// Which environment a `from_config` call is resolving versions for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // `Cloud` is reserved; cloud.rs does not yet read versions.
pub enum DeployEnv {
    Dev,
    Cloud,
}

const DEFAULT_SURREALDB_VERSION: &str = "v3.0.0";
const DEFAULT_SSP_VERSION: &str = "canary";
const DEFAULT_SCHEDULER_VERSION: &str = "canary";

/// How a single component (SSP or scheduler) should be launched in dev mode.
/// `Image` keeps the existing `docker run` path; `LocalBinary` spawns the
/// binary directly on the host. The `dev` runtime branches on this and
/// reshapes networking env vars accordingly (see `RuntimeUrls`).
#[derive(Debug, Clone)]
pub enum RuntimeSource {
    Image(String),          // tag, e.g. "canary"
    LocalBinary(PathBuf),   // absolute path to a host binary
}

impl RuntimeSource {
    pub fn is_local(&self) -> bool {
        matches!(self, RuntimeSource::LocalBinary(_))
    }
}

/// Resolved version config with all defaults applied.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ResolvedVersions {
    pub surrealdb: String,
    pub ssp: RuntimeSource,
    pub scheduler: RuntimeSource,
}

impl Default for ResolvedVersions {
    fn default() -> Self {
        Self {
            surrealdb: DEFAULT_SURREALDB_VERSION.to_string(),
            ssp: RuntimeSource::Image(DEFAULT_SSP_VERSION.to_string()),
            scheduler: RuntimeSource::Image(DEFAULT_SCHEDULER_VERSION.to_string()),
        }
    }
}

/// Resolve a `ComponentVersion` (or absent value with default tag) into a
/// `RuntimeSource`. Relative paths are anchored at `project_dir` so users
/// can write `../../target/debug/ssp-server` from inside `example/` without
/// caring about the cwd at invocation time.
fn resolve_component(
    cv: Option<&ComponentVersion>,
    default_tag: &str,
    project_dir: &Path,
) -> RuntimeSource {
    match cv {
        None => RuntimeSource::Image(default_tag.to_string()),
        Some(ComponentVersion::Tag(t)) => RuntimeSource::Image(t.clone()),
        Some(ComponentVersion::Path { path }) => {
            let resolved = if path.is_absolute() {
                path.clone()
            } else {
                project_dir.join(path)
            };
            RuntimeSource::LocalBinary(resolved)
        }
    }
}

impl ResolvedVersions {
    /// Resolve versions for the given deploy environment:
    /// - `surrealdb`: from top-level `surrealdb` field, else default
    /// - `version: "tag"` → sets both ssp & scheduler to "tag" for all envs
    /// - `version: { ssp, scheduler }` → individual control, applies to all envs
    /// - `version: { dev: ..., cloud: ... }` → per-env override; missing key for the
    ///   requested env falls back to defaults (same as no `version` field).
    ///
    /// Anchors any relative path entries against the current working dir.
    /// Use `from_config_with_dir` to pin them to the project dir explicitly.
    #[allow(dead_code)] // Public API surface; tests prefer from_config_with_dir.
    pub fn from_config(config: &Sp00kyConfig, env: DeployEnv) -> Self {
        Self::from_config_with_dir(config, env, Path::new("."))
    }

    /// As `from_config`, but anchors any relative `path:` entries against
    /// `project_dir`. Use this from the dev runtime so `../../target/...`
    /// in `example/sp00ky.yml` resolves regardless of where the user invoked
    /// `spooky dev`.
    pub fn from_config_with_dir(config: &Sp00kyConfig, env: DeployEnv, project_dir: &Path) -> Self {
        let surrealdb = config.resolved_surrealdb().version;

        let spec: Option<&VersionSpec> = match &config.version {
            None => None,
            Some(VersionConfig::Single(s)) => Some(s),
            Some(VersionConfig::PerEnvironment { dev, cloud }) => match env {
                DeployEnv::Dev => dev.as_ref(),
                DeployEnv::Cloud => cloud.as_ref(),
            },
        };

        let (ssp, scheduler) = match spec {
            Some(VersionSpec::All(v)) => (
                RuntimeSource::Image(v.clone()),
                RuntimeSource::Image(v.clone()),
            ),
            Some(VersionSpec::Individual { ssp, scheduler }) => (
                resolve_component(ssp.as_ref(), DEFAULT_SSP_VERSION, project_dir),
                resolve_component(scheduler.as_ref(), DEFAULT_SCHEDULER_VERSION, project_dir),
            ),
            None => (
                RuntimeSource::Image(DEFAULT_SSP_VERSION.to_string()),
                RuntimeSource::Image(DEFAULT_SCHEDULER_VERSION.to_string()),
            ),
        };

        Self { surrealdb, ssp, scheduler }
    }

    pub fn surrealdb_image(&self) -> String { format!("surrealdb/surrealdb:{}", self.surrealdb) }

    /// Image reference for the SSP, if it is launched from Docker.
    /// Returns `None` for `LocalBinary`.
    pub fn ssp_image(&self) -> Option<String> {
        match &self.ssp {
            RuntimeSource::Image(t) => Some(format!("mono424/spooky-ssp:{}", t)),
            RuntimeSource::LocalBinary(_) => None,
        }
    }

    /// Image reference for the scheduler, if it is launched from Docker.
    /// Returns `None` for `LocalBinary`.
    #[allow(dead_code)]
    pub fn scheduler_image(&self) -> Option<String> {
        match &self.scheduler {
            RuntimeSource::Image(t) => Some(format!("mono424/spooky-scheduler:{}", t)),
            RuntimeSource::LocalBinary(_) => None,
        }
    }

    /// Path to the SSP host binary, if configured. `None` for `Image`.
    #[allow(dead_code)]
    pub fn ssp_local_binary(&self) -> Option<&Path> {
        match &self.ssp {
            RuntimeSource::LocalBinary(p) => Some(p),
            RuntimeSource::Image(_) => None,
        }
    }

    /// Path to the scheduler host binary, if configured. `None` for `Image`.
    #[allow(dead_code)]
    pub fn scheduler_local_binary(&self) -> Option<&Path> {
        match &self.scheduler {
            RuntimeSource::LocalBinary(p) => Some(p),
            RuntimeSource::Image(_) => None,
        }
    }
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
    },
    #[serde(rename = "docker")]
    Docker {
        file: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workdir: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<String>,
    },
    #[serde(rename = "uv")]
    Uv {
        script: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workdir: Option<String>,
    },
}

/// Unified deploy configuration for all app types.
/// Port defaults depend on app type (8080 for backends, 3000 for frontends).
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AppDeployConfig {
    /// Dockerfile path (relative to sp00ky.yml)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dockerfile: Option<String>,
    /// Build context directory (relative to sp00ky.yml, defaults to project root)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Port the service listens on (no default — resolved by app type)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Resource allocation for the VM
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<BackendDeployResources>,
    /// Expose publicly via {slug}-{name}.fn.spky.cloud (backend only)
    #[serde(default)]
    pub expose: bool,
    /// Health check path for the scheduler to ping, e.g. "/health" (backend only)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub healthcheck: Option<String>,
    /// HTTP request timeout in seconds (backend only, default: 10)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u32>,
    /// Whether the frontend can override the timeout per-job (backend only)
    #[serde(default, rename = "timeoutOverridable", skip_serializing_if = "Option::is_none")]
    pub timeout_overridable: Option<bool>,
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

impl BackendDeployResources {
    pub fn validate(&self) -> Result<()> {
        if self.vcpus < 1 {
            bail!("resources.vcpus must be >= 1, got {}", self.vcpus);
        }
        if self.memory < 128 {
            bail!("resources.memory must be >= 128 MB, got {}", self.memory);
        }
        if self.disk < 1 {
            bail!("resources.disk must be >= 1 GB, got {}", self.disk);
        }
        Ok(())
    }
}

/// Unified application configuration — works for both backend and frontend apps.
#[derive(Debug, Deserialize, Serialize)]
pub struct AppConfig {
    /// App type: "backend" or "frontend" (required).
    #[serde(rename = "type")]
    pub app_type: AppType,

    // ── Backend-specific fields ─────────────────────────────────────────
    /// "cloud" (default) or "external" — whether this backend is deployed to
    /// Sp00ky Cloud or self-hosted at `baseUrl`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hosting: Option<HostingMode>,
    /// Path to the OpenAPI specification file (required for backends).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<String>,
    /// Base URL for the backend service (required when hosting is "external").
    #[serde(default, rename = "baseUrl", skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthConfig>,
    /// Trigger method (required for backends).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<BackendMethod>,

    // ── Shared fields ───────────────────────────────────────────────────
    /// Dev server configuration (npm, docker, uv, or raw command).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev: Option<BackendDevConfig>,
    /// Deployment configuration (dockerfile, port, resources, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deploy: Option<AppDeployConfig>,
    /// Environment variable configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<EnvConfig>,
}

impl AppConfig {
    pub fn resolved_hosting(&self) -> HostingMode {
        self.hosting.clone().unwrap_or_default()
    }

    /// Resolve the deploy port, falling back to type-specific defaults.
    pub fn deploy_port(&self) -> u16 {
        self.deploy.as_ref()
            .and_then(|d| d.port)
            .unwrap_or(match self.app_type {
                AppType::Backend => 8080,
                AppType::Frontend => 3000,
            })
    }

    pub fn validate(&self, name: &str) -> Result<()> {
        match self.app_type {
            AppType::Backend => {
                if self.spec.is_none() {
                    bail!("Backend app '{}' is missing required field 'spec'", name);
                }
                if self.method.is_none() {
                    bail!("Backend app '{}' is missing required field 'method'", name);
                }
                if self.resolved_hosting() == HostingMode::External && self.base_url.is_none() {
                    bail!("Backend app '{}' has hosting 'external' but no baseUrl was provided", name);
                }
            }
            AppType::Frontend => {
                if let Some(ref deploy) = self.deploy {
                    if deploy.dockerfile.is_none() {
                        bail!("Frontend app '{}' has 'deploy' but is missing 'dockerfile'", name);
                    }
                }
            }
        }
        if let Some(ref deploy) = self.deploy {
            if let Some(ref resources) = deploy.resources {
                resources.validate()
                    .context(format!("Invalid resources for app '{}'", name))?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AuthConfig {
    #[serde(rename = "type")]
    pub auth_type: AuthType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct BackendMethod {
    #[serde(rename = "type")]
    pub method_type: MethodType,
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
        mode: Some(DeployMode::Singlenode),
        surrealdb: None,
        version: None,
        schema: None,
        apps: Default::default(),
        buckets: Default::default(),
        client_types: Default::default(),
        deployment: None,
        cloud_api: None,
        migration_engine: None,
        surrealkit: None,
        log_level: None,
        ref_mode: None,
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

        for (name, app) in &config.apps {
            if app.app_type != AppType::Backend {
                continue;
            }
            self.process_backend(name, app, base_dir)?;
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

    fn process_backend(&mut self, backend_name: &str, app_config: &AppConfig, base_dir: &Path) -> Result<()> {
        println!("Processing backend config: {}", backend_name);

        let method = app_config.method.as_ref()
            .context(format!("Backend '{}' is missing 'method' field", backend_name))?;
        let spec = app_config.spec.as_ref()
            .context(format!("Backend '{}' is missing 'spec' field", backend_name))?;

        // 1. Append Schema - resolve path relative to sp00ky.yml
        let schema_path = base_dir.join(&method.schema);
        let schema_content = fs::read_to_string(&schema_path)
            .context(format!("Failed to read backend schema: {:?}", schema_path))?;

        self.schema_appends.push('\n');
        self.schema_appends
            .push_str(&format!("-- Backend Schema: {}\n", backend_name));
        self.schema_appends.push_str(&schema_content);
        println!("  + Appended schema from {:?}", schema_path);

        // 2. Parse OpenAPI Spec - resolve path relative to sp00ky.yml
        let spec_path = base_dir.join(spec);
        let spec_content = fs::read_to_string(&spec_path)
            .context(format!("Failed to read openapi spec: {:?}", spec_path))?;

        let openapi: OpenAPI =
            serde_yaml::from_str(&spec_content).context("Failed to parse openapi spec")?;

        let mut backend_def = BackendDefinition {
            routes: BTreeMap::new(),
            outbox_table: method.table.clone(),
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

#[cfg(test)]
mod version_tests {
    use super::*;

    fn parse(yaml: &str) -> Sp00kyConfig {
        serde_yaml::from_str(yaml).expect("yaml parse")
    }

    fn try_parse(yaml: &str) -> std::result::Result<Sp00kyConfig, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }

    /// Helper for tests that only care about the tag form. Panics if either
    /// side resolved to a `LocalBinary` — those tests must call
    /// `resolve_full` and pattern-match on the `RuntimeSource`.
    fn resolve(yaml: &str, env: DeployEnv) -> (String, String) {
        let r = resolve_full(yaml, env);
        let unwrap_tag = |rs: RuntimeSource| match rs {
            RuntimeSource::Image(t) => t,
            RuntimeSource::LocalBinary(p) => panic!("expected Image, got LocalBinary({:?})", p),
        };
        (unwrap_tag(r.ssp), unwrap_tag(r.scheduler))
    }

    fn resolve_full(yaml: &str, env: DeployEnv) -> ResolvedVersions {
        let cfg = parse(yaml);
        ResolvedVersions::from_config(&cfg, env)
    }

    #[test]
    fn flat_string_applies_to_both_envs() {
        assert_eq!(resolve("version: canary\n", DeployEnv::Dev), ("canary".into(), "canary".into()));
        assert_eq!(resolve("version: canary\n", DeployEnv::Cloud), ("canary".into(), "canary".into()));
    }

    #[test]
    fn per_service_object_applies_to_both_envs() {
        let yaml = "version: { ssp: a, scheduler: b }\n";
        assert_eq!(resolve(yaml, DeployEnv::Dev), ("a".into(), "b".into()));
        assert_eq!(resolve(yaml, DeployEnv::Cloud), ("a".into(), "b".into()));
    }

    #[test]
    fn per_service_partial_fills_with_defaults() {
        let yaml = "version: { ssp: a }\n";
        assert_eq!(resolve(yaml, DeployEnv::Dev), ("a".into(), "canary".into()));
    }

    #[test]
    fn per_env_strings() {
        let yaml = "version:\n  dev: dev\n  cloud: canary\n";
        assert_eq!(resolve(yaml, DeployEnv::Dev), ("dev".into(), "dev".into()));
        assert_eq!(resolve(yaml, DeployEnv::Cloud), ("canary".into(), "canary".into()));
    }

    #[test]
    fn per_env_with_per_service() {
        let yaml = "version:\n  dev: { ssp: x, scheduler: y }\n  cloud: canary\n";
        assert_eq!(resolve(yaml, DeployEnv::Dev), ("x".into(), "y".into()));
        assert_eq!(resolve(yaml, DeployEnv::Cloud), ("canary".into(), "canary".into()));
    }

    #[test]
    fn per_env_missing_dev_falls_back_to_defaults() {
        let yaml = "version:\n  cloud: canary\n";
        assert_eq!(resolve(yaml, DeployEnv::Dev), ("canary".into(), "canary".into()));
        assert_eq!(resolve(yaml, DeployEnv::Cloud), ("canary".into(), "canary".into()));
    }

    #[test]
    fn no_version_uses_defaults() {
        assert_eq!(resolve("apps: {}\n", DeployEnv::Dev), ("canary".into(), "canary".into()));
    }

    #[test]
    fn unknown_key_in_per_service_errors() {
        // `bogus` isn't a valid VersionSpec::Individual field; deny_unknown_fields rejects it.
        let yaml = "version: { ssp: x, bogus: y }\n";
        assert!(try_parse(yaml).is_err());
    }

    #[test]
    fn mixed_keys_error() {
        // Not a subset of {dev, cloud}, falls through to VersionSpec::Individual which rejects `dev`.
        let yaml = "version: { ssp: x, dev: y }\n";
        assert!(try_parse(yaml).is_err());
    }

    #[test]
    fn round_trip_serialize_flat_string() {
        let cfg = parse("version: canary\n");
        let out = serde_yaml::to_string(&cfg.version).unwrap();
        assert_eq!(out.trim(), "canary");
    }

    #[test]
    fn round_trip_serialize_per_env() {
        let cfg = parse("version:\n  dev: dev\n  cloud: canary\n");
        let out = serde_yaml::to_string(&cfg.version).unwrap();
        // Round-trips back to a parseable PerEnvironment structure.
        let reparsed: VersionConfig = serde_yaml::from_str(&out).unwrap();
        match reparsed {
            VersionConfig::PerEnvironment { dev, cloud } => {
                assert!(matches!(dev, Some(VersionSpec::All(ref s)) if s == "dev"));
                assert!(matches!(cloud, Some(VersionSpec::All(ref s)) if s == "canary"));
            }
            _ => panic!("expected PerEnvironment, got {:?}", reparsed),
        }
    }

    #[test]
    fn per_service_path_resolves_to_local_binary() {
        let yaml = "version:\n  ssp: { path: ./target/debug/ssp-server }\n  scheduler: { path: /abs/path/scheduler }\n";
        let cfg = parse(yaml);
        let r = ResolvedVersions::from_config_with_dir(&cfg, DeployEnv::Dev, Path::new("/proj"));

        match &r.ssp {
            RuntimeSource::LocalBinary(p) => {
                // Relative path is anchored at project_dir.
                assert_eq!(p, &PathBuf::from("/proj/./target/debug/ssp-server"));
            }
            other => panic!("expected LocalBinary, got {:?}", other),
        }
        match &r.scheduler {
            RuntimeSource::LocalBinary(p) => {
                // Absolute path is preserved verbatim.
                assert_eq!(p, &PathBuf::from("/abs/path/scheduler"));
            }
            other => panic!("expected LocalBinary, got {:?}", other),
        }
    }

    #[test]
    fn per_service_mixed_tag_and_path() {
        let yaml = "version:\n  ssp: { path: ./target/debug/ssp-server }\n  scheduler: canary\n";
        let r = resolve_full(yaml, DeployEnv::Dev);
        assert!(matches!(r.ssp, RuntimeSource::LocalBinary(_)));
        match &r.scheduler {
            RuntimeSource::Image(t) => assert_eq!(t, "canary"),
            other => panic!("expected Image(canary), got {:?}", other),
        }
    }

    #[test]
    fn per_env_with_path_in_dev() {
        let yaml = "version:\n  dev: { ssp: { path: ./target/debug/ssp-server }, scheduler: { path: ./target/debug/scheduler } }\n  cloud: canary\n";
        let cfg = parse(yaml);
        let dev = ResolvedVersions::from_config_with_dir(&cfg, DeployEnv::Dev, Path::new("/proj"));
        let cloud = ResolvedVersions::from_config_with_dir(&cfg, DeployEnv::Cloud, Path::new("/proj"));

        assert!(matches!(dev.ssp, RuntimeSource::LocalBinary(_)));
        assert!(matches!(dev.scheduler, RuntimeSource::LocalBinary(_)));
        assert!(matches!(cloud.ssp, RuntimeSource::Image(ref t) if t == "canary"));
        assert!(matches!(cloud.scheduler, RuntimeSource::Image(ref t) if t == "canary"));
    }

    #[test]
    fn helpers_split_image_and_path() {
        let yaml = "version:\n  ssp: { path: ./bin/ssp }\n  scheduler: canary\n";
        let cfg = parse(yaml);
        let r = ResolvedVersions::from_config_with_dir(&cfg, DeployEnv::Dev, Path::new("/proj"));

        assert!(r.ssp_image().is_none());
        assert_eq!(r.ssp_local_binary().unwrap(), Path::new("/proj/./bin/ssp"));

        assert_eq!(r.scheduler_image().as_deref(), Some("mono424/spooky-scheduler:canary"));
        assert!(r.scheduler_local_binary().is_none());
    }
}
