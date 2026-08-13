//! Merge-key behaviour through the REAL registration path.
//!
//! `merge_key::compute` is unit-tested against synthetic plans, but the property
//! that actually protects users depends on the whole pipeline: surql -> converter
//! -> permission injection -> key. A permission whose `$auth.id` reference is
//! lowered somewhere these tests do not reach would produce equal keys for two
//! different users, and one would be served the other's rows.
//!
//! So these go through `prepare_registration_dbsp` with real PERMISSIONS text,
//! exactly as the SSP does at `ssp-node/src/node.rs:810`.

use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};

fn perms(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(t, p)| (t.to_string(), p.to_string()))
        .collect()
}

/// Register `surql` as `auth_id` would, mirroring what `fn::query::register`
/// injects server-side: `params.auth.id` and `params.access`.
fn key_for(
    surql: &str,
    auth_id: &str,
    access: &str,
    permissions: &HashMap<String, String>,
) -> String {
    let config = json!({
        "id": format!("_00_query:{auth_id}-{access}"),
        "surql": surql,
        "clientId": "sess",
        "ttl": "10m",
        "lastActiveAt": "2024-01-01T00:00:00Z",
        "params": { "auth": { "id": auth_id }, "access": access },
    });
    let links: HashMap<String, HashMap<String, String>> = HashMap::new();
    let opaque: HashMap<String, BTreeSet<String>> = HashMap::new();
    ssp::service::view::prepare_registration_dbsp(config, permissions, &links, &opaque)
        .expect("registration should prepare")
        .merge_key
}

#[test]
fn two_sessions_of_one_user_share_a_key() {
    let p = perms(&[("thread", "author.id = $auth.id")]);
    let a = key_for("SELECT * FROM thread", "user:alice", "account", &p);
    let b = key_for("SELECT * FROM thread", "user:alice", "account", &p);
    assert_eq!(a, b, "same user, same query: one graph");
}

#[test]
fn two_users_never_share_a_key_on_an_auth_scoped_query() {
    // The security property. If this ever fails, merged views serve Bob the
    // rows Alice is allowed to see.
    let p = perms(&[("thread", "author.id = $auth.id")]);
    let alice = key_for("SELECT * FROM thread", "user:alice", "account", &p);
    let bob = key_for("SELECT * FROM thread", "user:bob", "account", &p);
    assert_ne!(alice, bob, "auth-scoped permission must separate identities");
}

#[test]
fn two_users_do_share_a_key_on_a_public_query() {
    // `PERMISSIONS true` injects nothing, so the plan dereferences no auth
    // param and every user (plus anon) collapses onto one graph. This is the
    // largest memory win in the change.
    let p = perms(&[("thread", "true")]);
    let alice = key_for("SELECT * FROM thread", "user:alice", "account", &p);
    let bob = key_for("SELECT * FROM thread", "user:bob", "account", &p);
    let anon = key_for("SELECT * FROM thread", "", "", &p);
    assert_eq!(alice, bob, "public query shares one graph across users");
    assert_eq!(alice, anon, "and with anonymous sessions");
}

#[test]
fn an_access_gated_permission_separates_access_levels_but_not_users() {
    let p = perms(&[("thread", "$access = \"account\"")]);
    let alice = key_for("SELECT * FROM thread", "user:alice", "account", &p);
    let bob = key_for("SELECT * FROM thread", "user:bob", "account", &p);
    let public = key_for("SELECT * FROM thread", "user:carol", "public", &p);
    assert_eq!(alice, bob, "same access level shares");
    assert_ne!(alice, public, "different access level must not share");
}

#[test]
fn a_mixed_permission_still_separates_identities() {
    // The common real shape: public rows OR your own. Because one arm
    // dereferences `$auth.id`, the key must stay per-identity.
    let p = perms(&[(
        "thread",
        "published = true OR ($access = \"account\" AND author.id = $auth.id)",
    )]);
    let alice = key_for("SELECT * FROM thread", "user:alice", "account", &p);
    let bob = key_for("SELECT * FROM thread", "user:bob", "account", &p);
    assert_ne!(alice, bob);
}

#[test]
fn different_user_params_never_share_a_key() {
    // Params the USER supplied are part of the plan's operands, so two
    // different filters must not merge even for one identity.
    let p = perms(&[("thread", "true")]);
    let mk = |surql: &str| key_for(surql, "user:alice", "account", &p);
    assert_ne!(
        mk("SELECT * FROM thread WHERE status = 'open'"),
        mk("SELECT * FROM thread WHERE status = 'closed'")
    );
}

#[test]
fn a_permission_change_changes_the_key() {
    // Injection is part of the computation, so a schema edit must not leave
    // pre-change registrations merged with post-change ones.
    let before = key_for(
        "SELECT * FROM thread",
        "user:alice",
        "account",
        &perms(&[("thread", "true")]),
    );
    let after = key_for(
        "SELECT * FROM thread",
        "user:alice",
        "account",
        &perms(&[("thread", "published = true")]),
    );
    assert_ne!(before, after);
}

#[test]
fn the_key_is_independent_of_the_client_chosen_query_id() {
    // Whole premise of merging below the id: two sessions pick different
    // `_00_query` ids for the same computation and must still collapse.
    let p = perms(&[("thread", "true")]);
    let base = json!({
        "surql": "SELECT * FROM thread",
        "clientId": "sess",
        "ttl": "10m",
        "lastActiveAt": "2024-01-01T00:00:00Z",
        "params": { "auth": { "id": "user:alice" }, "access": "account" },
    });
    let with_id = |id: &str| {
        let mut c = base.clone();
        c.as_object_mut()
            .unwrap()
            .insert("id".into(), Value::String(id.into()));
        let links: HashMap<String, HashMap<String, String>> = HashMap::new();
        let opaque: HashMap<String, BTreeSet<String>> = HashMap::new();
        ssp::service::view::prepare_registration_dbsp(c, &p, &links, &opaque)
            .expect("prepare")
            .merge_key
    };
    assert_eq!(with_id("_00_query:aaa"), with_id("_00_query:bbb"));
}
