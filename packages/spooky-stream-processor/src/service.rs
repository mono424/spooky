use crate::engine::view::{IdTree, Operator};
use crate::{converter, sanitizer, MaterializedViewUpdate, QueryPlan};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

pub mod ingest {
    use super::*;

    /// Prepares a record for ingestion by normalizing and hashing it.
    pub fn prepare(record: Value) -> (Value, String) {
        let clean_record = sanitizer::normalize_record(record);
        let hash = blake3::hash(clean_record.to_string().as_bytes())
            .to_hex()
            .to_string();
        (clean_record, hash)
    }
}

pub mod view {
    use super::*;

    /// Parsed registration request data
    pub struct RegistrationData {
        pub plan: QueryPlan,
        pub safe_params: Option<Value>,
        pub metadata: Value,
    }

    /// Prepares a view registration request.
    /// Returns the QueryPlan, sanitized params, and a JSON object containing the metadata
    /// needed for the `_spooky_incantation` table.
    pub fn prepare_registration(config: Value) -> Result<RegistrationData> {
        let id = config
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing or invalid 'id'"))?
            .to_string();

        // Handle potential case differences in keys if coming from different sources,
        // but generally we expect camelCase from JSON payloads or standard keys.
        // Sidecar uses "surrealQL" (or "surreal_ql" mapped), module uses "surrealQL".
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

        // Parse Query Plan
        let root_op_val = converter::convert_surql_to_dbsp(&surreal_ql)
            .or_else(|_| serde_json::from_str(&surreal_ql).map_err(anyhow::Error::from))
            .map_err(|_| anyhow!("Invalid Query Plan"))?;

        let root_op: Operator =
            serde_json::from_value(root_op_val).map_err(|_| anyhow!("Invalid Operator JSON"))?;

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
            "params": params, // Store original params or safe params? Module stored original. Sidecar stored safe. Let's store safe for consistency? Or original?
                              // Module: params from config. Sidecar: safe_params.
                              // Let's stick to safe_params for consistency if reasonable.
                              // Actually module stored `params` (raw).
                              // Use safe_params to be cleaner.
            "safe_params": safe_params_val,
            "ttl": ttl,
            "lastActiveAt": last_active_at
        });

        Ok(RegistrationData {
            plan,
            safe_params,
            metadata,
        })
    }

    pub fn default_result(id: &str) -> MaterializedViewUpdate {
        let empty_hash = blake3::hash(&[]).to_hex().to_string();
        MaterializedViewUpdate {
            query_id: id.to_string(),
            result_hash: empty_hash.clone(),
            result_ids: vec![],
            tree: IdTree {
                hash: empty_hash,
                children: None,
                leaves: Some(vec![]),
            },
        }
    }
}
