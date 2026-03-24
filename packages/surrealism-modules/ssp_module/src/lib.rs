// TODO: re-enable when ssp API stabilizes
// This module is disabled for v1. The sp00ky_stream_processor API has changed
// and the types referenced here (Circuit, ViewUpdate, MaterializedViewUpdate, etc.)
// no longer exist in their previous form.

use serde_json::{json, Value};
use surrealism::surrealism;

// mod persistence;

// use sp00ky_stream_processor::{
//     engine::{
//         circuit::{dto::BatchEntry, Circuit},
//         types::Operation,
//     },
//     MaterializedViewUpdate,
// };
// use smol_str::SmolStr;

#[surrealism]
fn ingest(
    _table: String,
    _operation: String,
    _id: String,
    _record: Value,
) -> Result<Value, &'static str> {
    // TODO: re-enable when ssp API stabilizes
    Ok(Value::Null)
}

#[surrealism]
fn version(_args: Value) -> Result<Value, &'static str> {
    Ok(json!("0.2.0-disabled"))
}

#[surrealism]
fn register_view(_config: Value) -> Result<Value, &'static str> {
    // TODO: re-enable when ssp API stabilizes
    Ok(Value::Null)
}

#[surrealism]
fn unregister_view(_id: String) -> Result<Value, &'static str> {
    // TODO: re-enable when ssp API stabilizes
    Ok(Value::Null)
}

#[surrealism]
fn reset(_val: Value) -> Result<Value, &'static str> {
    // TODO: re-enable when ssp API stabilizes
    Ok(Value::Null)
}

#[surrealism]
fn save_state(_val: Value) -> Result<Value, &'static str> {
    // TODO: re-enable when ssp API stabilizes
    Ok(Value::Null)
}
