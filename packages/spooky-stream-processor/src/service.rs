use crate::engine::view::{IdTree, Operator};
use crate::{converter, sanitizer, MaterializedViewUpdate, QueryPlan};
use anyhow::{anyhow, Result};
use simd_json::{json, OwnedValue as Value};
use simd_json::prelude::*; // for .into_trait_impls and value traits

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
        // 1. Convert SURQL to generic Value (now OwnedValue)
        let root_op_val = converter::convert_surql_to_dbsp(&surreal_ql)
             .or_else(|_| {
                 // Fallback: Parse directly from string using simd-json
                 // We need bytes for simd-json
                 let mut bytes = surreal_ql.clone().into_bytes();
                 simd_json::to_owned_value(&mut bytes).map_err(anyhow::Error::from)
             })
             .map_err(|_| anyhow!("Invalid Query Plan"))?;

        // 2. Deserialize Value into Operator Struct
        // Operator uses OwnedValue fields.
        // We can use simd_json::serde::from_owned_value or standard serde if OwnedValue implements deserializer.
        // simd_json 0.13 usually requires explicit conversion for struct deserialization if not using from_slice directly.
        // But since Operator derives Deserialize, and OwnedValue implements Deserializer...
        let root_op: Operator = simd_json::serde::from_owned_value(root_op_val)
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
