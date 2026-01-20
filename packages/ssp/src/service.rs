use crate::engine::update::ViewResultFormat;
use crate::engine::view::{Operator, SpookyValue};
use crate::{converter, sanitizer, MaterializedViewUpdate, QueryPlan};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tracing::instrument;

fn hash_value_recursive_blake3(v: &Value, hasher: &mut blake3::Hasher) {
    match v {
        Value::Null => {
            hasher.update(&[0]);
        }
        Value::Bool(b) => {
            hasher.update(&[1]);
            hasher.update(&[*b as u8]);
        }
        Value::Number(n) => {
            hasher.update(&[2]);
            if let Some(f) = n.as_f64() {
                hasher.update(&f.to_be_bytes());
            } else {
                // strict fallback if it somehow isn't an f64 compatible number, though in JS/JSON it mostly is
                hasher.update(n.to_string().as_bytes());
            }
        }
        Value::String(s) => {
            hasher.update(&[3]);
            hasher.update(s.as_bytes());
        }
        Value::Array(arr) => {
            hasher.update(&[4]);
            for item in arr {
                hash_value_recursive_blake3(item, hasher);
            }
        }
        Value::Object(obj) => {
            hasher.update(&[5]);
            // Deterministic hashing by sorting keys?
            // To match current recursive approach in view.rs which iterates straight:
            // We stick to simple iteration unless strict determinism across reloads is needed.
            // Given the previous code didn't sort, we won't sort here to maintain behavior relative to previous logic,
            // BUT `service.rs` uses `hash_value_recursive` which previously used `to_string` on numbers.
            // The prompt asks for optimization.
            for (k, v) in obj {
                hasher.update(k.as_bytes());
                hash_value_recursive_blake3(v, hasher);
            }
        }
    }
}

pub mod ingest {
    use super::*;

    /// Prepares a record for ingestion by normalizing and hashing it.
    #[instrument(skip(record))]
    pub fn prepare(record: Value) -> (SpookyValue, String) {
        let clean_record = sanitizer::normalize_record(record);
        let mut hasher = blake3::Hasher::new();
        hash_value_recursive_blake3(&clean_record, &mut hasher);
        let hash = hasher.finalize().to_hex().to_string();
        (SpookyValue::from(clean_record), hash)
    }

    /// Prepares a batch of records, optionally in parallel.
    #[instrument(skip(records))]
    pub fn prepare_batch(records: Vec<Value>) -> Vec<(SpookyValue, String)> {
        #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
        {
            use rayon::prelude::*;
            records.into_par_iter().map(prepare).collect()
        }

        #[cfg(any(target_arch = "wasm32", not(feature = "parallel")))]
        {
            records.into_iter().map(prepare).collect()
        }
    }

    /// Fast preparation: Skips normalization/sanitization for high throughput.
    pub fn prepare_fast(record: Value) -> (SpookyValue, String) {
        let mut hasher = blake3::Hasher::new();
        hash_value_recursive_blake3(&record, &mut hasher);
        let hash = hasher.finalize().to_hex().to_string();
        (SpookyValue::from(record), hash)
    }
}

pub mod view {
    use super::*;

    use crate::engine::metadata::VersionStrategy;

    /// Parsed registration request data
    pub struct RegistrationData {
        pub plan: QueryPlan,
        pub safe_params: Option<Value>,
        pub metadata: Value,
        pub format: Option<ViewResultFormat>,
        pub strategy: Option<VersionStrategy>,
    }

    /// Prepares a view registration request.
    #[instrument(skip(config))]
    pub fn prepare_registration(config: Value) -> Result<RegistrationData> {
        let id = config
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing or invalid 'id'"))?
            .to_string();

        let surreal_ql = config
            .get("surrealQL")
            .or_else(|| config.get("surreal_ql"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing or invalid 'surrealQL'"))?
            .to_string();

        let client_id = config
            .get("clientId")
            .or_else(|| config.get("client_id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing or invalid 'clientId'"))?
            .to_string();

        let ttl = config
            .get("ttl")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing or invalid 'ttl'"))?
            .to_string();

        let last_active_at = config
            .get("lastActiveAt")
            .or_else(|| config.get("last_active_at"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing or invalid 'lastActiveAt'"))?
            .to_string();

        let params = config.get("params").cloned().unwrap_or(json!({}));

        // Parse optional format field (defaults to Flat if not specified)
        let format = config
            .get("format")
            .or_else(|| config.get("resultFormat"))
            .and_then(|v| v.as_str())
            .and_then(|s| match s.to_lowercase().as_str() {
                "streaming" => Some(ViewResultFormat::Streaming),
                "tree" => Some(ViewResultFormat::Tree),
                "flat" => Some(ViewResultFormat::Flat),
                _ => None,
            });

        // Parse optional strategy
        let strategy = config
            .get("strategy")
            .or_else(|| config.get("versionStrategy"))
            .and_then(|v| v.as_str())
            .and_then(|s| match s.to_lowercase().as_str() {
                "optimistic" => Some(VersionStrategy::Optimistic),
                "explicit" => Some(VersionStrategy::Explicit),
                "hashbased" | "hash_based" => Some(VersionStrategy::HashBased),
                "none" => Some(VersionStrategy::None),
                _ => None,
            });

        // Parse Query Plan
        // 1. Convert SURQL to generic Value
        let root_op_val = converter::convert_surql_to_dbsp(&surreal_ql)
            .or_else(|_| {
                // Fallback: Parse directly from string using serde_json
                serde_json::from_str::<Value>(&surreal_ql).map_err(anyhow::Error::from)
            })
            .map_err(|_| anyhow!("Invalid Query Plan"))?;

        // 2. Deserialize Value into Operator Struct
        let root_op: Operator = serde_json::from_value(root_op_val)
            .map_err(|e| anyhow!("Invalid Operator JSON: {}", e))?;

        let safe_params = sanitizer::parse_params(params.clone());
        let safe_params_val = safe_params.clone().unwrap_or(json!({}));

        let plan = QueryPlan {
            id: id.clone(),
            root: root_op,
        };

        let metadata = json!({
            "id": id,
            "clientId": client_id,
            "surrealQL": surreal_ql,
            "params": params,
            "safe_params": safe_params_val,
            "ttl": ttl,
            "lastActiveAt": last_active_at
        });

        Ok(RegistrationData {
            plan,
            safe_params,
            metadata,
            format,
            strategy,
        })
    }

    #[instrument]
    pub fn default_result(id: &str) -> MaterializedViewUpdate {
        let empty_hash = blake3::hash(&[]).to_hex().to_string();
        MaterializedViewUpdate {
            query_id: id.to_string(),
            result_hash: empty_hash,
            result_data: vec![], // Empty flat array
        }
    }
}
