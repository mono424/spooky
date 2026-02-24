use crate::{converter, sanitizer};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

pub mod view {
    use super::*;

    /// Parsed registration data using new DBSP types
    pub struct DbspRegistrationData {
        pub plan: crate::operator::plan::QueryPlan,
        pub safe_params: Option<Value>,
        pub metadata: Value,
        pub format: Option<crate::circuit::view::OutputFormat>,
    }

    /// Prepares a view registration request using DBSP types.
    pub fn prepare_registration_dbsp(config: Value) -> Result<DbspRegistrationData> {
        use crate::circuit::view::OutputFormat;
        use crate::operator::plan::{OperatorPlan, QueryPlan};

        let id = config
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing or invalid 'id'"))?
            .to_string();

        let surreal_ql = config
            .get("surql")
            .or_else(|| config.get("surreal_ql"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing or invalid 'surql'"))?
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

        let format = config
            .get("format")
            .or_else(|| config.get("resultFormat"))
            .and_then(|v| v.as_str())
            .and_then(|s| match s.to_lowercase().as_str() {
                "streaming" => Some(OutputFormat::Streaming),
                "tree" => Some(OutputFormat::Tree),
                "flat" => Some(OutputFormat::Flat),
                _ => None,
            });

        let root_op_val = converter::convert_surql_to_dbsp(&surreal_ql)
            .or_else(|_| {
                serde_json::from_str::<Value>(&surreal_ql).map_err(anyhow::Error::from)
            })
            .map_err(|_| anyhow!("Invalid Query Plan"))?;

        let root_op: OperatorPlan = serde_json::from_value(root_op_val)
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
            "sql": surreal_ql,
            "params": params,
            "safe_params": safe_params_val,
            "ttl": ttl,
            "lastActiveAt": last_active_at
        });

        Ok(DbspRegistrationData {
            plan,
            safe_params,
            metadata,
            format,
        })
    }
}
