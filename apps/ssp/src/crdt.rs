//! Server-side CRDT merge now lives in the portable core (`ssp_node::crdt`,
//! over the `Db` port). Re-exported so existing `crate::crdt::…` paths work.
//! `allow_list_from_env` stays shell-side (the core never reads the env).
pub use ssp_node::crdt::{ApplyRequest, ApplyResponse, CrdtAllowList, CrdtCache};

use std::collections::HashMap;
use tracing::warn;

/// Build the CRDT allow-list from `SPKY_CRDT_FIELDS`
/// (`{"thread":["title","content"], ...}`). Unset/invalid → permissive (dev).
pub fn allow_list_from_env() -> CrdtAllowList {
    match std::env::var("SPKY_CRDT_FIELDS") {
        Ok(s) if !s.is_empty() => match serde_json::from_str::<HashMap<String, Vec<String>>>(&s) {
            Ok(map) => CrdtAllowList::from_map(map),
            Err(e) => {
                warn!(error = %e, "SPKY_CRDT_FIELDS is not valid JSON, falling back to permissive mode");
                CrdtAllowList::permissive()
            }
        },
        _ => {
            warn!("SPKY_CRDT_FIELDS not set, /crdt/apply running in permissive mode");
            CrdtAllowList::permissive()
        }
    }
}
