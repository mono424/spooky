use anyhow::{Context, Result};
use openapiv3::OpenAPI;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct SpookyConfig {
    pub backends: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct BackendConfig {
    pub spec: String,
    pub method: BackendMethod,
}

#[derive(Debug, Deserialize)]
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
}

impl BackendProcessor {
    pub fn new() -> Self {
        Self {
            schema_appends: String::new(),
            backend_definitions: BTreeMap::new(),
        }
    }

    pub fn process(&mut self, config_path: &Path) -> Result<()> {
        let config_str = fs::read_to_string(config_path)
            .context(format!("Failed to read spooky config: {:?}", config_path))?;

        let config: SpookyConfig =
            serde_yaml::from_str(&config_str).context("Failed to parse spooky config")?;

        let base_dir = config_path.parent().unwrap_or(Path::new("."));

        for backend_rel_path in config.backends {
            let backend_path = base_dir.join(&backend_rel_path);
            self.process_backend(&backend_path)?;
        }

        Ok(())
    }

    fn process_backend(&mut self, backend_path: &Path) -> Result<()> {
        println!("Processing backend config: {:?}", backend_path);

        let backend_str = fs::read_to_string(backend_path)
            .context(format!("Failed to read backend config: {:?}", backend_path))?;

        let backend_config: BackendConfig =
            serde_yaml::from_str(&backend_str).context("Failed to parse backend config")?;

        let backend_dir = backend_path.parent().unwrap_or(Path::new("."));

        // 1. Append Schema
        let schema_path = backend_dir.join(&backend_config.method.schema);
        let schema_content = fs::read_to_string(&schema_path)
            .context(format!("Failed to read backend schema: {:?}", schema_path))?;

        self.schema_appends.push('\n');
        self.schema_appends
            .push_str(&format!("-- Backend Schema: {:?}\n", backend_path));
        self.schema_appends.push_str(&schema_content);
        println!("  + Appended schema from {:?}", schema_path);

        // 2. Parse OpenAPI Spec
        let spec_path = backend_dir.join(&backend_config.spec);
        let spec_content = fs::read_to_string(&spec_path)
            .context(format!("Failed to read openapi spec: {:?}", spec_path))?;

        let openapi: OpenAPI =
            serde_yaml::from_str(&spec_content).context("Failed to parse openapi spec")?;

        // Extract backend name from directory name (as per requirement "api comes from the folder name")
        // assuming backend_path is something like ".../api/backend.yml" -> "api"
        let backend_name = backend_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let mut backend_def = BackendDefinition {
            routes: BTreeMap::new(),
            outbox_table: backend_config.method.table,
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

        self.backend_definitions.insert(backend_name, backend_def);
        println!("  + Parsed OpenAPI spec from {:?}", spec_path);

        Ok(())
    }
}
