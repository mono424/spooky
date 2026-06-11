use crate::{converter, permission_inject, sanitizer};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use web_time::Instant;

pub mod view {
    use super::*;

    /// Parsed registration data using new DBSP types
    pub struct DbspRegistrationData {
        pub plan: crate::operator::plan::QueryPlan,
        pub safe_params: Option<Value>,
        pub metadata: Value,
        pub format: Option<crate::circuit::view::OutputFormat>,
        /// Time (ms) spent converting/parsing the surql into a plan and
        /// injecting permissions. Surfaced to DevTools as the SSP "parse" phase.
        pub parse_ms: f64,
    }

    /// Prepares a view registration request using DBSP types.
    ///
    /// Runs the converter on the user's surql, then injects each table's
    /// permission predicate per scan via `permission_inject::inject_permissions`.
    /// Errors abort registration so callers can surface them (typically as
    /// HTTP 400) with the offending table named.
    pub fn prepare_registration_dbsp(
        config: Value,
        permissions: &HashMap<String, String>,
    ) -> Result<DbspRegistrationData> {
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

        // Time the convert→plan→permission-inject path as the SSP "parse" phase.
        let parse_start = Instant::now();

        let root_op_val = converter::convert_surql_to_dbsp(&surreal_ql)
            .or_else(|_| {
                serde_json::from_str::<Value>(&surreal_ql).map_err(anyhow::Error::from)
            })
            .map_err(|_| anyhow!("Invalid Query Plan"))?;

        let mut root_op: OperatorPlan = serde_json::from_value(root_op_val)
            .map_err(|e| anyhow!("Invalid Operator JSON: {}", e))?;

        let safe_params = sanitizer::parse_params(params.clone());
        let safe_params_val = safe_params.clone().unwrap_or(json!({}));

        // Extract the user-scoped auth identity from the injected params.
        // `fn::query::register` injects `params.auth.id = <string>$auth.id`
        // server-side, so by the time we get here `safe_params.auth.id`
        // is the authenticated caller's user record id (as a string like
        // "user:abc"). We carry this forward as `auth_id` on `_00_query`
        // and `_00_list_ref` so cross-session LIVE delivery on
        // `_00_list_ref` can gate on `auth_id = $auth.id` (stable
        // across re-auth / reconnect) instead of the per-connection
        // `session::id()`.
        //
        // Note: we cannot read this from a top-level `authId` field on
        // the registration payload because SurrealDB's `http::post`
        // silently strips object keys that aren't in the runtime's
        // recognized set for that call site. The `params.auth.id` path
        // is the only channel that already survives that filtering.
        let auth_id = safe_params_val
            .get("auth")
            .and_then(|a| a.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        permission_inject::inject_permissions(&mut root_op, permissions, safe_params.as_ref())?;

        let parse_ms = parse_start.elapsed().as_secs_f64() * 1000.0;

        let plan = QueryPlan {
            id: id.clone(),
            root: root_op,
        };

        let metadata = json!({
            "id": id,
            "clientId": client_id,
            "authId": auth_id,
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
            parse_ms,
        })
    }
}

#[cfg(test)]
mod start_window_isolation_tests {
    use super::view::prepare_registration_dbsp;
    use crate::circuit::view::OutputFormat;
    use crate::circuit::{Circuit, Record};
    use serde_json::json;
    use std::collections::HashMap;

    // Mirrors register_view_handler: surql -> prepare_registration_dbsp ->
    // add_query_with_auth(Streaming) -> the returned ViewDelta.records is what the
    // SSP writes into _00_list_ref. For START 50 over 120 rows it must be the
    // window [50,100), not the top-50.
    #[test]
    fn ssp_register_applies_start_offset() {
        let mut recs = vec![];
        for i in 0..120u32 {
            let id = format!("g{:03}", i);
            recs.push(Record::new(
                "game",
                &id,
                json!({ "id": format!("game:{}", id), "database": "game_database:c1",
                        "sort_index": i, "date": "2024-01-01T00:00:00Z" }),
            ));
        }
        let mut circuit = Circuit::new();
        circuit.load(recs);

        let mut perms = HashMap::new();
        perms.insert("game".to_string(), "true".to_string());

        let payload = json!({
            "id": "_00_query:test",
            "surql": "SELECT * FROM game WHERE database = $database ORDER BY sort_index asc, date desc, id asc LIMIT 50 START 50;",
            "params": { "database": "game_database:c1" },
            "clientId": "c", "ttl": "10m", "lastActiveAt": "", "format": "streaming"
        });
        let data = prepare_registration_dbsp(payload, &perms).expect("prep");
        let update = circuit
            .add_query_with_auth(data.plan, data.safe_params, Some(OutputFormat::Streaming), String::new())
            .expect("delta");
        let mut got = update.records.clone();
        got.sort();
        eprintln!("records.len={} first={:?} last={:?}", got.len(), got.first(), got.last());
        assert_eq!(got.len(), 50, "window size");
        assert_eq!(got.first().map(String::as_str), Some("game:g050"), "window must START at offset 50");
        assert_eq!(got.last().map(String::as_str), Some("game:g099"));
    }
}

#[cfg(test)]
mod start_window_register_before_ingest_tests {
    use super::view::prepare_registration_dbsp;
    use crate::circuit::view::OutputFormat;
    use crate::circuit::{Circuit, Change, ChangeSet};
    use serde_json::json;
    use std::collections::HashMap;

    // RUNTIME ORDER: the SSP registers the view, THEN games stream in via ingest.
    // (The passing isolation test loads data first, then registers.) For START 50
    // the view must still settle to the window [50,100), not the top-50.
    #[test]
    fn ssp_offset_window_correct_when_registered_before_ingest() {
        let mut circuit = Circuit::new();
        let mut perms = HashMap::new();
        perms.insert("game".to_string(), "true".to_string());
        let payload = json!({
            "id": "_00_query:test",
            "surql": "SELECT * FROM game WHERE database = $database ORDER BY sort_index asc, date desc, id asc LIMIT 50 START 50;",
            "params": { "database": "game_database:c1" },
            "clientId": "c", "ttl": "10m", "lastActiveAt": "", "format": "streaming"
        });
        let data = prepare_registration_dbsp(payload, &perms).expect("prep");
        circuit.add_query_with_auth(data.plan, data.safe_params, Some(OutputFormat::Streaming), String::new());

        let changes: Vec<Change> = (0..160u32)
            .map(|i| {
                let id = format!("g{:03}", i);
                Change::create(
                    "game",
                    &id,
                    json!({ "id": format!("game:{}", id), "database": "game_database:c1",
                            "sort_index": i, "date": "2024-01-01T00:00:00Z" }),
                )
            })
            .collect();
        circuit.step(ChangeSet { changes });

        let view = circuit.get_view("_00_query:test").expect("view");
        let mut recs: Vec<String> = view.cache.keys().cloned().collect();
        recs.sort();
        eprintln!("len={} first={:?} last={:?}", recs.len(), recs.first(), recs.last());
        assert_eq!(recs.len(), 50, "window size");
        assert_eq!(recs.first().map(String::as_str), Some("game:g050"), "must START at offset 50");
    }
}
