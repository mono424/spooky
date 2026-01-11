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
