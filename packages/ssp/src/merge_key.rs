//! Merge key: an identity for the *computation* a registration performs.
//!
//! Two registrations that produce the same merge key provably maintain the same
//! row set, so they can share one operator graph instead of paying for one each
//! (the dominant cost, see `Graph::state_bytes`).
//!
//! # Why the plan alone is not the key
//!
//! Plans REFERENCE params, they never bake them. `author.id = $auth.id` lowers
//! to `Predicate::Eq { field: "author.id", value: {"$param": "auth.id"} }`
//! (`converter.rs`), and that reference is resolved per row at evaluation time
//! against the view's params (`operator::filter::resolve_predicate_value`).
//! So two DIFFERENT users registering the same surql produce byte-identical
//! plans while maintaining different row sets. Keying on the plan alone would
//! merge them and serve one user the other's rows.
//!
//! The key is therefore the plan PLUS the value of every param the plan
//! actually dereferences. A useful property falls out for free: a plan that
//! never dereferences `auth.id` does not admit it to the key, so every user
//! (and anon) shares one graph for public queries. That is the single biggest
//! memory win available and it needs no special case.
//!
//! # Failing closed
//!
//! A param reference this module MISSES makes two users' keys collide, which is
//! a cross-user row leak rather than a crash. Two defences:
//!
//! 1. Every `match` here is exhaustive with no wildcard arm, so adding a
//!    `Predicate` or `OperatorPlan` variant is a COMPILE ERROR rather than a
//!    silent leak. Do not "fix" such an error with `_ => {}`.
//! 2. Anything unresolvable falls back to mixing the entire params blob into
//!    the key, which degrades merging to per-identity (merely wasteful) instead
//!    of over-merging (unsafe).

use crate::operator::plan::{OperatorPlan, Projection};
use crate::operator::predicate::Predicate;
use serde_json::Value;
use ssp_protocol::snapshot_hash::canonical_json;
use std::collections::BTreeSet;

const KEY_PREFIX: &str = "mk1:";

/// Collect every registration param path the plan dereferences.
///
/// `parent.`-prefixed paths are EXCLUDED: they are per-row correlation for
/// subqueries, not registration params (`filter::resolve_predicate_value`
/// strips the prefix and resolves against the row context). Including them
/// would stop every `.related()` view from ever merging, and they carry no
/// registration identity.
pub fn collect_param_refs(plan: &OperatorPlan) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    walk_plan(plan, &mut out);
    out
}

fn push_path(out: &mut BTreeSet<String>, path: &str) {
    if path.strip_prefix("parent.").is_some() || path.is_empty() {
        return;
    }
    out.insert(path.to_string());
}

/// A `$param` reference on the RHS of a comparison, as produced by the
/// converter: `{"$param": "auth.id"}`.
fn push_value_ref(out: &mut BTreeSet<String>, value: &Value) {
    if let Some(obj) = value.as_object() {
        if let Some(p) = obj.get("$param").and_then(|v| v.as_str()) {
            push_path(out, p);
        }
    }
}

fn walk_predicate(pred: &Predicate, out: &mut BTreeSet<String>) {
    match pred {
        // RHS may be a `$param` reference.
        Predicate::Eq { value, .. }
        | Predicate::Neq { value, .. }
        | Predicate::Gt { value, .. }
        | Predicate::Gte { value, .. }
        | Predicate::Lt { value, .. }
        | Predicate::Lte { value, .. } => push_value_ref(out, value),

        // LHS param name is stored separately from the RHS, and the RHS may
        // ITSELF be a `$param` (see `filter::check_predicate_recursive`).
        Predicate::ParamEq { param, value }
        | Predicate::ParamNeq { param, value }
        | Predicate::ParamGt { param, value }
        | Predicate::ParamGte { param, value }
        | Predicate::ParamLt { param, value }
        | Predicate::ParamLte { param, value } => {
            push_path(out, param);
            push_value_ref(out, value);
        }

        Predicate::And { predicates } | Predicate::Or { predicates } => {
            for p in predicates {
                walk_predicate(p, out);
            }
        }

        // `prefix` is a baked literal, never a param.
        Predicate::Prefix { .. } => {}
        Predicate::True | Predicate::False => {}
    }
}

fn walk_plan(plan: &OperatorPlan, out: &mut BTreeSet<String>) {
    match plan {
        OperatorPlan::Scan { .. } => {}
        OperatorPlan::Filter { input, predicate } => {
            walk_predicate(predicate, out);
            walk_plan(input, out);
        }
        OperatorPlan::Join { left, right, .. }
        | OperatorPlan::SemiJoin { left, right, .. }
        | OperatorPlan::AntiJoin { left, right, .. }
        | OperatorPlan::Union { left, right } => {
            walk_plan(left, out);
            walk_plan(right, out);
        }
        OperatorPlan::Distinct { input } => walk_plan(input, out),
        OperatorPlan::Limit { input, .. } => walk_plan(input, out),
        OperatorPlan::Project { input, projections } => {
            walk_plan(input, out);
            for proj in projections {
                match proj {
                    // Neither selects through a param.
                    Projection::All | Projection::Field { .. } => {}
                    Projection::Subquery { plan, .. } => walk_plan(plan, out),
                }
            }
        }
    }
}

/// Resolve a dotted path against the registration params.
///
/// Mirrors `filter::resolve_param_lookup` for the shapes that actually occur
/// here (`auth.id`, `access`). `None` means unresolvable, which drives the
/// fail-closed fallback rather than being treated as null.
fn resolve<'a>(params: Option<&'a Value>, path: &str) -> Option<&'a Value> {
    let mut cur = params?;
    for seg in path.split('.') {
        cur = cur.get(seg)?;
    }
    Some(cur)
}

/// Identity of the computation this (plan, params) pair performs.
///
/// Registrations sharing a key may share one operator graph.
pub fn compute(plan: &OperatorPlan, params: Option<&Value>) -> String {
    let mut hasher = blake3::Hasher::new();

    let plan_json = serde_json::to_value(plan).unwrap_or(Value::Null);
    hasher.update(b"plan:");
    hasher.update(&canonical_json(&plan_json));

    let refs = collect_param_refs(plan);
    let mut unresolved = false;
    for path in &refs {
        match resolve(params, path) {
            Some(v) => {
                hasher.update(b"\x00bind:");
                hasher.update(path.as_bytes());
                hasher.update(b"=");
                hasher.update(&canonical_json(v));
            }
            // The plan dereferences something the params do not carry. We
            // cannot prove two such registrations compute the same thing, so
            // fall back to the whole blob: never merge across any param
            // difference at all.
            None => unresolved = true,
        }
    }

    if unresolved {
        hasher.update(b"\x00fallback:");
        hasher.update(&canonical_json(params.unwrap_or(&Value::Null)));
    }

    format!("{KEY_PREFIX}{}", hasher.finalize().to_hex())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plan_from(v: Value) -> OperatorPlan {
        serde_json::from_value(v).expect("plan")
    }

    /// `WHERE author.id = $auth.id` — the auth-scoped shape.
    fn auth_scoped() -> OperatorPlan {
        plan_from(json!({
            "op": "filter",
            "input": { "op": "scan", "table": "thread" },
            "predicate": {
                "type": "eq",
                "field": "author.id",
                "value": { "$param": "auth.id" }
            }
        }))
    }

    /// `WHERE published = true` — references no registration param.
    fn public() -> OperatorPlan {
        plan_from(json!({
            "op": "filter",
            "input": { "op": "scan", "table": "thread" },
            "predicate": { "type": "eq", "field": "published", "value": true }
        }))
    }

    fn params(auth: &str, access: &str) -> Value {
        json!({ "auth": { "id": auth }, "access": access })
    }

    #[test]
    fn same_identity_two_sessions_merge() {
        let p = auth_scoped();
        // Two sessions of one user differ only in their client-chosen query id,
        // which is not an input here at all.
        assert_eq!(
            compute(&p, Some(&params("user:alice", "account"))),
            compute(&p, Some(&params("user:alice", "account")))
        );
    }

    #[test]
    fn different_identities_must_not_merge_on_an_auth_scoped_query() {
        // THE safety property. A collision here serves Bob Alice's rows.
        let p = auth_scoped();
        assert_ne!(
            compute(&p, Some(&params("user:alice", "account"))),
            compute(&p, Some(&params("user:bob", "account")))
        );
    }

    #[test]
    fn different_identities_do_merge_on_an_auth_free_query() {
        // The whole point of keying on dereferenced params rather than on
        // identity: a public query costs ONE graph across every user and anon.
        let p = public();
        assert_eq!(
            compute(&p, Some(&params("user:alice", "account"))),
            compute(&p, Some(&params("user:bob", "")))
        );
        assert_eq!(
            compute(&p, Some(&params("user:alice", "account"))),
            compute(&p, Some(&json!({})))
        );
    }

    #[test]
    fn access_only_permission_merges_within_an_access_level() {
        let p = plan_from(json!({
            "op": "filter",
            "input": { "op": "scan", "table": "thread" },
            "predicate": { "type": "parameq", "param": "access", "value": "account" }
        }));
        assert_eq!(
            compute(&p, Some(&params("user:alice", "account"))),
            compute(&p, Some(&params("user:bob", "account"))),
            "same access level shares one graph"
        );
        assert_ne!(
            compute(&p, Some(&params("user:alice", "account"))),
            compute(&p, Some(&params("user:bob", "public"))),
            "different access levels must not share"
        );
    }

    #[test]
    fn parent_correlated_refs_do_not_block_merging() {
        // `parent.` is per-row correlation, not registration identity. If it
        // entered the key, no `.related()` view would ever merge.
        let p = plan_from(json!({
            "op": "filter",
            "input": { "op": "scan", "table": "comment" },
            "predicate": {
                "type": "eq",
                "field": "thread",
                "value": { "$param": "parent.id" }
            }
        }));
        assert!(collect_param_refs(&p).is_empty());
        assert_eq!(
            compute(&p, Some(&params("user:alice", "account"))),
            compute(&p, Some(&params("user:bob", "account")))
        );
    }

    #[test]
    fn refs_are_found_inside_subqueries_and_join_arms() {
        let p = plan_from(json!({
            "op": "project",
            "input": { "op": "scan", "table": "thread" },
            "projections": [{
                "type": "subquery",
                "alias": "author",
                "plan": {
                    "op": "filter",
                    "input": { "op": "scan", "table": "user" },
                    "predicate": {
                        "type": "eq",
                        "field": "id",
                        "value": { "$param": "auth.id" }
                    }
                }
            }]
        }));
        assert!(collect_param_refs(&p).contains("auth.id"));
        assert_ne!(
            compute(&p, Some(&params("user:alice", "account"))),
            compute(&p, Some(&params("user:bob", "account"))),
            "a ref reachable only through a subquery still separates identities"
        );
    }

    #[test]
    fn an_unresolvable_ref_fails_closed() {
        // Plan dereferences `auth.id` but params carry none: we cannot prove
        // equality, so the whole blob enters the key and nothing merges across
        // any difference.
        let p = auth_scoped();
        let a = json!({ "access": "account", "other": 1 });
        let b = json!({ "access": "account", "other": 2 });
        assert_ne!(compute(&p, Some(&a)), compute(&p, Some(&b)));
        assert_eq!(compute(&p, Some(&a)), compute(&p, Some(&a)));
    }

    #[test]
    fn a_different_plan_never_shares_a_key() {
        assert_ne!(
            compute(&auth_scoped(), Some(&params("user:alice", "account"))),
            compute(&public(), Some(&params("user:alice", "account")))
        );
    }
}
