//! Permission policy: implicit WHERE injection.
//!
//! The SSP runs a parallel incremental view of SurrealDB. SurrealDB applies a
//! table's `PERMISSIONS FOR select WHERE <expr>` rule on every query, so a
//! plain `SELECT * FROM thread` returns only the rows the caller is allowed
//! to see. The SSP, on its own, has no idea that rule exists — its scans
//! emit deltas for every row in the table.
//!
//! This module fixes that: a `PermissionRegistry` holds the parsed predicate
//! for each table, and `rewrite_plan` walks an `OperatorPlan` and wraps every
//! `Scan { table }` in a `Filter { predicate }` carrying the table's permission
//! rule. The rewrite is logged per-scan so it's transparent what the SSP added.
//!
//! Fail-closed by design: if a table has no registered permission, or the
//! permission references `$auth.*` but the registration didn't supply auth
//! params, the injected predicate is `Predicate::False` so the SSP never
//! shows more rows than SurrealDB would.

use std::collections::HashMap;

use serde_json::Value;
use tracing::{info, warn};

use crate::operator::plan::{OperatorPlan, Projection};
use crate::operator::predicate::Predicate;

/// Per-table permission predicates. Loaded from SurrealDB at SSP boot.
#[derive(Debug, Default, Clone)]
pub struct PermissionRegistry {
    by_table: HashMap<String, Predicate>,
}

impl PermissionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, table: impl Into<String>, predicate: Predicate) {
        self.by_table.insert(table.into(), predicate);
    }

    pub fn get(&self, table: &str) -> Option<&Predicate> {
        self.by_table.get(table)
    }

    pub fn len(&self) -> usize {
        self.by_table.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_table.is_empty()
    }
}

/// One entry in the rewrite report — recorded per `Scan` we touched.
///
/// We only emit an entry when the registry had a permission for the table.
/// Tables with no registered permission are passed through unchanged, so the
/// fail-closed decision lives at boot-time (where `NONE` / unparseable maps
/// to `Predicate::False`) rather than here.
#[derive(Debug, Clone)]
pub struct RewriteEntry {
    pub table: String,
    /// The original filter predicate sitting directly above the scan, if any.
    pub original_filter: Option<Predicate>,
    /// The permission predicate we injected (after auth-context fail-closed).
    pub injected: Predicate,
    /// True if the predicate was downgraded to `False` because required
    /// `$auth` params weren't supplied at registration time.
    pub auth_missing: bool,
}

#[derive(Debug, Default, Clone)]
pub struct RewriteReport {
    pub entries: Vec<RewriteEntry>,
}

impl RewriteReport {
    /// Emit one structured `info!` per scan rewrite. Target `ssp::policy`
    /// so it can be filtered independently in the logs.
    pub fn log(&self, view_id: &str) {
        for e in &self.entries {
            info!(
                target: "ssp::policy",
                view_id = %view_id,
                table = %e.table,
                injected = ?e.injected,
                original_filter = ?e.original_filter,
                auth_missing = e.auth_missing,
                "applied permission rewrite"
            );
        }
    }
}

/// Walk `plan` and wrap every `Scan` with the registered permission predicate.
/// Returns a report describing what was injected, for logging.
pub fn rewrite_plan(
    plan: &mut OperatorPlan,
    registry: &PermissionRegistry,
    params: Option<&Value>,
) -> RewriteReport {
    let mut report = RewriteReport::default();
    rewrite_node(plan, registry, params, &mut report);
    report
}

fn rewrite_node(
    plan: &mut OperatorPlan,
    registry: &PermissionRegistry,
    params: Option<&Value>,
    report: &mut RewriteReport,
) {
    match plan {
        OperatorPlan::Scan { table } => {
            let table_name = table.clone();
            let Some(entry) = build_entry(&table_name, None, registry, params) else {
                // No permission registered for this table — pass through. Tables
                // the boot loader couldn't introspect (or never saw, like meta
                // tables) stay untouched. Fail-closed lives at boot time.
                return;
            };
            let injected = entry.injected.clone();
            report.entries.push(entry);

            // Replace this Scan node with `Filter { predicate, input: Scan }`.
            let scan = OperatorPlan::Scan {
                table: table_name,
            };
            *plan = OperatorPlan::Filter {
                input: Box::new(scan),
                predicate: injected,
            };
        }
        OperatorPlan::Filter { input, predicate } => {
            // If the immediate child is a Scan, fold the permission predicate
            // into the existing filter via AND, so we don't end up with two
            // stacked Filter nodes per table.
            if let OperatorPlan::Scan { table } = input.as_ref() {
                let table_name = table.clone();
                let original = predicate.clone();
                let Some(entry) =
                    build_entry(&table_name, Some(original.clone()), registry, params)
                else {
                    return;
                };
                let injected = entry.injected.clone();
                report.entries.push(entry);

                *predicate = and_predicates(original, injected);
                // input is still the Scan — nothing else to recurse into.
                return;
            }
            // Otherwise descend; some other operator will own the scan.
            rewrite_node(input, registry, params, report);
        }
        OperatorPlan::Join { left, right, .. } => {
            rewrite_node(left, registry, params, report);
            rewrite_node(right, registry, params, report);
        }
        OperatorPlan::Project { input, projections } => {
            rewrite_node(input, registry, params, report);
            for proj in projections.iter_mut() {
                if let Projection::Subquery { plan, .. } = proj {
                    rewrite_node(plan, registry, params, report);
                }
            }
        }
        OperatorPlan::Limit { input, .. } => {
            rewrite_node(input, registry, params, report);
        }
    }
}

fn build_entry(
    table: &str,
    original_filter: Option<Predicate>,
    registry: &PermissionRegistry,
    params: Option<&Value>,
) -> Option<RewriteEntry> {
    let mut injected = registry.get(table).cloned()?;

    // Auth-context fail-closed: predicate references $auth.* but the caller
    // didn't supply an `auth` param — we have no identity to filter on.
    let mut auth_missing = false;
    if injected.references_auth() && !params_have_auth(params) {
        warn!(
            target: "ssp::policy",
            table = %table,
            "permission references $auth but registration params lack auth — denying"
        );
        injected = Predicate::False;
        auth_missing = true;
    }

    Some(RewriteEntry {
        table: table.to_string(),
        original_filter,
        injected,
        auth_missing,
    })
}

fn params_have_auth(params: Option<&Value>) -> bool {
    params
        .and_then(|p| p.get("auth"))
        .map(|v| !v.is_null())
        .unwrap_or(false)
}

/// AND two predicates, flattening when one (or both) are already `And`.
/// Drops `True` operands and short-circuits on `False`.
fn and_predicates(a: Predicate, b: Predicate) -> Predicate {
    if matches!(a, Predicate::False) || matches!(b, Predicate::False) {
        return Predicate::False;
    }
    let mut out: Vec<Predicate> = Vec::new();
    push_and_operand(&mut out, a);
    push_and_operand(&mut out, b);
    match out.len() {
        0 => Predicate::True,
        1 => out.into_iter().next().unwrap(),
        _ => Predicate::And { predicates: out },
    }
}

fn push_and_operand(out: &mut Vec<Predicate>, p: Predicate) {
    match p {
        Predicate::True => {}
        Predicate::And { predicates } => {
            for inner in predicates {
                push_and_operand(out, inner);
            }
        }
        other => out.push(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operator::plan::{OperatorPlan, Projection, QueryPlan};
    use crate::types::Path;
    use serde_json::json;

    fn registry_with(table: &str, p: Predicate) -> PermissionRegistry {
        let mut r = PermissionRegistry::new();
        r.set(table, p);
        r
    }

    fn auth_eq() -> Predicate {
        Predicate::Eq {
            field: Path::new("author.id"),
            value: json!({ "$param": "auth.id" }),
        }
    }

    #[test]
    fn scan_is_wrapped_in_filter() {
        let mut plan = OperatorPlan::Scan {
            table: "thread".into(),
        };
        let registry = registry_with("thread", auth_eq());
        let params = json!({ "auth": { "id": "user:a" } });
        let report = rewrite_plan(&mut plan, &registry, Some(&params));

        assert_eq!(report.entries.len(), 1);
        assert!(!report.entries[0].auth_missing);
        match plan {
            OperatorPlan::Filter { input, predicate } => {
                assert!(matches!(*input, OperatorPlan::Scan { .. }));
                assert!(matches!(predicate, Predicate::Eq { .. }));
            }
            _ => panic!("expected Filter at root"),
        }
    }

    #[test]
    fn existing_filter_is_anded_not_double_wrapped() {
        let user_pred = Predicate::Eq {
            field: Path::new("active"),
            value: json!(true),
        };
        let mut plan = OperatorPlan::Filter {
            input: Box::new(OperatorPlan::Scan {
                table: "thread".into(),
            }),
            predicate: user_pred,
        };
        let registry = registry_with("thread", auth_eq());
        let params = json!({ "auth": { "id": "user:a" } });
        rewrite_plan(&mut plan, &registry, Some(&params));

        match plan {
            OperatorPlan::Filter { predicate, .. } => match predicate {
                Predicate::And { predicates } => {
                    assert_eq!(predicates.len(), 2);
                }
                other => panic!("expected And, got {:?}", other),
            },
            _ => panic!("expected Filter at root"),
        }
    }

    #[test]
    fn missing_auth_falls_to_false() {
        let mut plan = OperatorPlan::Scan {
            table: "thread".into(),
        };
        let registry = registry_with("thread", auth_eq());
        let report = rewrite_plan(&mut plan, &registry, None);

        assert!(report.entries[0].auth_missing);
        match plan {
            OperatorPlan::Filter { predicate, .. } => {
                assert!(matches!(predicate, Predicate::False));
            }
            _ => panic!("expected Filter"),
        }
    }

    #[test]
    fn missing_registry_entry_passes_through() {
        // Tables with no registered permission stay untouched. Fail-closed for
        // unknown tables lives at boot time (boot loader registers
        // Predicate::False for unparseable / NONE permissions).
        let mut plan = OperatorPlan::Scan {
            table: "thread".into(),
        };
        let registry = PermissionRegistry::new();
        let report = rewrite_plan(&mut plan, &registry, None);

        assert!(report.entries.is_empty());
        assert!(matches!(plan, OperatorPlan::Scan { .. }));
    }

    #[test]
    fn subquery_plan_is_rewritten() {
        let inner_scan = OperatorPlan::Scan {
            table: "comment".into(),
        };
        let projection = Projection::Subquery {
            alias: "comments".into(),
            plan: Box::new(inner_scan),
            parent_key: None,
        };
        let mut plan = OperatorPlan::Project {
            input: Box::new(OperatorPlan::Scan {
                table: "thread".into(),
            }),
            projections: vec![projection],
        };

        let mut registry = PermissionRegistry::new();
        registry.set("thread", Predicate::True);
        registry.set("comment", Predicate::True);
        let report = rewrite_plan(&mut plan, &registry, None);

        // We should have visited both scans.
        let visited: Vec<_> = report.entries.iter().map(|e| e.table.as_str()).collect();
        assert!(visited.contains(&"thread"));
        assert!(visited.contains(&"comment"));
    }

    #[test]
    fn true_predicate_does_not_clutter_existing_filter() {
        let user_pred = Predicate::Eq {
            field: Path::new("active"),
            value: json!(true),
        };
        let mut plan = OperatorPlan::Filter {
            input: Box::new(OperatorPlan::Scan {
                table: "thread".into(),
            }),
            predicate: user_pred.clone(),
        };
        let registry = registry_with("thread", Predicate::True);
        rewrite_plan(&mut plan, &registry, None);

        match plan {
            OperatorPlan::Filter { predicate, .. } => {
                // and_predicates drops True, so the user predicate remains alone.
                assert!(matches!(predicate, Predicate::Eq { .. }));
            }
            _ => panic!("expected Filter"),
        }
    }

    #[test]
    fn query_plan_helper_smoke() {
        let mut plan = QueryPlan {
            id: "v1".into(),
            root: OperatorPlan::Scan {
                table: "thread".into(),
            },
        };
        let registry = registry_with("thread", Predicate::True);
        rewrite_plan(&mut plan.root, &registry, None);
        assert!(matches!(plan.root, OperatorPlan::Filter { .. }));
    }
}
