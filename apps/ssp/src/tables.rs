//! Per-user `_00_list_ref` table helpers now live in the portable core
//! (`ssp_node::tables`, over the `Db` port). Re-exported here so existing
//! `crate::tables::…` call sites keep working while the handlers that use them
//! migrate into the core.
pub use ssp_node::tables::*;
