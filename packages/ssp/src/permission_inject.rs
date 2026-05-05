//! Per-Scan permission injection.
//!
//! For each `Scan { table }` in a registered query's plan, look up the table's
//! raw `PERMISSIONS FOR select WHERE <expr>` text, parse it through the same
//! `convert_surql_to_dbsp` pipeline that handles user queries, and AND-fold the
//! resulting predicate into the existing `Filter` directly above the scan (or
//! wrap the scan in a new `Filter`). The SSP runs as root and bypasses
//! SurrealDB's own permission enforcement, so this is what stops it from
//! over-sharing rows back to clients.
//!
//! Failures are surfaced at registration time. Callers map them to HTTP 400
//! with the offending table named in the error so the client can fix its
//! schema or supply the missing `auth` param. Default-deny: a table with no
//! registered permission is rejected, except `_00_*` meta tables (which the
//! boot loader filters out and the SSP itself accesses directly).

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::converter;
use crate::operator::plan::{OperatorPlan, Projection};
use crate::operator::predicate::Predicate;

/// Walk `plan` and inject each scanned table's permission predicate. Errors
/// abort registration; partial injection is never visible.
pub fn inject_permissions(
    plan: &mut OperatorPlan,
    perms: &HashMap<String, String>,
    params: Option<&Value>,
) -> Result<()> {
    inject_node(plan, perms, params)
}

fn inject_node(
    plan: &mut OperatorPlan,
    perms: &HashMap<String, String>,
    params: Option<&Value>,
) -> Result<()> {
    match plan {
        OperatorPlan::Scan { table } => {
            let table_name = table.clone();
            let Some(injected) = build_predicate(&table_name, perms, params)? else {
                return Ok(());
            };
            let scan = OperatorPlan::Scan { table: table_name };
            *plan = OperatorPlan::Filter {
                input: Box::new(scan),
                predicate: injected,
            };
        }
        OperatorPlan::Filter { input, predicate } => {
            if let OperatorPlan::Scan { table } = input.as_ref() {
                let table_name = table.clone();
                let Some(injected) = build_predicate(&table_name, perms, params)? else {
                    return Ok(());
                };
                let original = std::mem::replace(predicate, Predicate::True);
                *predicate = and_predicates(original, injected);
                return Ok(());
            }
            inject_node(input, perms, params)?;
        }
        OperatorPlan::Join { left, right, .. }
        | OperatorPlan::SemiJoin { left, right, .. }
        | OperatorPlan::AntiJoin { left, right, .. }
        | OperatorPlan::Union { left, right } => {
            inject_node(left, perms, params)?;
            inject_node(right, perms, params)?;
        }
        OperatorPlan::Project { input, projections } => {
            inject_node(input, perms, params)?;
            for proj in projections.iter_mut() {
                if let Projection::Subquery { plan, .. } = proj {
                    inject_node(plan, perms, params)?;
                }
            }
        }
        OperatorPlan::Limit { input, .. } | OperatorPlan::Distinct { input } => {
            inject_node(input, perms, params)?;
        }
    }
    Ok(())
}

/// Look up the permission text for `table` and return the predicate to AND in.
/// Returns `Ok(None)` for permissions that resolve to `true` (no filtering
/// needed) or for `_00_*` meta tables that the SSP accesses directly. Returns
/// `Err` for default-deny, unsupported constructs, missing `$auth`, or
/// converter parse failures.
fn build_predicate(
    table: &str,
    perms: &HashMap<String, String>,
    params: Option<&Value>,
) -> Result<Option<Predicate>> {
    let raw = match perms.get(table) {
        Some(t) => t.trim(),
        None => {
            if table.starts_with("_00_") {
                return Ok(None);
            }
            return Err(anyhow!(
                "table `{table}` has no PERMISSIONS clause registered; default-deny"
            ));
        }
    };

    if raw.eq_ignore_ascii_case("true") {
        return Ok(None);
    }
    if raw.eq_ignore_ascii_case("false") {
        return Err(anyhow!("table `{table}` permission is FALSE; access denied"));
    }

    if raw.contains("$auth") && !params_have_auth(params) {
        return Err(anyhow!(
            "permission for `{table}` requires $auth but registration params lack it"
        ));
    }

    if let Some(snippet) = unsupported_construct(raw) {
        return Err(anyhow!(
            "permission for `{table}` contains unsupported construct: {snippet}"
        ));
    }

    let synthetic = format!("SELECT * FROM {table} WHERE {raw}");
    let plan_val = converter::convert_surql_to_dbsp(&synthetic)
        .map_err(|e| anyhow!("permission for `{table}` failed to parse: {e}"))?;

    let parsed: OperatorPlan = serde_json::from_value(plan_val).map_err(|e| {
        anyhow!("permission for `{table}` deserialize error: {e}")
    })?;

    match parsed {
        OperatorPlan::Filter { input, predicate } => match *input {
            OperatorPlan::Scan { .. } => Ok(Some(predicate)),
            _ => Err(anyhow!(
                "permission for `{table}` produced a non-flat plan (likely identifier-vs-identifier comparison)"
            )),
        },
        _ => Err(anyhow!(
            "permission for `{table}` produced a non-filter plan (likely a join or subquery)"
        )),
    }
}

fn params_have_auth(params: Option<&Value>) -> bool {
    params
        .and_then(|p| p.get("auth"))
        .map(|v| !v.is_null())
        .unwrap_or(false)
}

/// Substring scan for permission constructs the SSP cannot represent. Returns
/// the matched snippet for error messages. We use case-insensitive matching
/// because SurrealDB doesn't care about keyword case.
fn unsupported_construct(raw: &str) -> Option<&'static str> {
    let lower = raw.to_lowercase();
    const NEEDLES: &[&str] = &[
        "in (select",
        "exists (select",
        "(select value",
        "$parent",
    ];
    for needle in NEEDLES {
        if lower.contains(needle) {
            return Some(needle);
        }
    }
    None
}

/// AND two predicates, flattening when either is already an `And` and dropping
/// `True` operands. Short-circuits to `False` when either side is `False`.
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
    use crate::operator::plan::{OperatorPlan, Projection};
    use crate::types::Path;
    use serde_json::json;

    fn perms_with(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(t, p)| (t.to_string(), p.to_string())).collect()
    }

    #[test]
    fn missing_table_is_default_deny() {
        let mut plan = OperatorPlan::Scan {
            table: "thread".into(),
        };
        let perms = HashMap::new();
        let err = inject_permissions(&mut plan, &perms, None).unwrap_err();
        assert!(err.to_string().contains("thread"));
        assert!(err.to_string().contains("default-deny"));
    }

    #[test]
    fn meta_tables_pass_through_when_missing() {
        let mut plan = OperatorPlan::Scan {
            table: "_00_query".into(),
        };
        let perms = HashMap::new();
        inject_permissions(&mut plan, &perms, None).unwrap();
        assert!(matches!(plan, OperatorPlan::Scan { .. }));
    }

    #[test]
    fn false_permission_errors() {
        let mut plan = OperatorPlan::Scan {
            table: "thread".into(),
        };
        let perms = perms_with(&[("thread", "false")]);
        let err = inject_permissions(&mut plan, &perms, None).unwrap_err();
        assert!(err.to_string().contains("FALSE"));
    }

    #[test]
    fn true_permission_is_noop() {
        let mut plan = OperatorPlan::Scan {
            table: "thread".into(),
        };
        let perms = perms_with(&[("thread", "true")]);
        inject_permissions(&mut plan, &perms, None).unwrap();
        assert!(matches!(plan, OperatorPlan::Scan { .. }));
    }

    #[test]
    fn auth_required_but_missing_errors() {
        let mut plan = OperatorPlan::Scan {
            table: "thread".into(),
        };
        let perms = perms_with(&[("thread", "author.id = $auth.id")]);
        let err = inject_permissions(&mut plan, &perms, None).unwrap_err();
        assert!(err.to_string().contains("$auth"));
    }

    #[test]
    fn unsupported_subquery_errors() {
        let mut plan = OperatorPlan::Scan {
            table: "thread".into(),
        };
        let perms = perms_with(&[(
            "thread",
            "$auth.id IN (SELECT VALUE in FROM collaborates_on)",
        )]);
        let params = json!({"auth": {"id": "user:a"}});
        let err = inject_permissions(&mut plan, &perms, Some(&params)).unwrap_err();
        assert!(err.to_string().contains("unsupported construct"));
    }

    #[test]
    fn simple_eq_wraps_scan_in_filter() {
        let mut plan = OperatorPlan::Scan {
            table: "thread".into(),
        };
        let perms = perms_with(&[("thread", "author.id = $auth.id")]);
        let params = json!({"auth": {"id": "user:a"}});
        inject_permissions(&mut plan, &perms, Some(&params)).unwrap();

        match plan {
            OperatorPlan::Filter { input, predicate } => {
                assert!(matches!(*input, OperatorPlan::Scan { .. }));
                match predicate {
                    Predicate::Eq { field, value } => {
                        assert_eq!(field.segments(), &["author".to_string(), "id".to_string()]);
                        assert_eq!(value, json!({"$param": "auth.id"}));
                    }
                    other => panic!("expected Eq, got {:?}", other),
                }
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
        let perms = perms_with(&[("thread", "author.id = $auth.id")]);
        let params = json!({"auth": {"id": "user:a"}});
        inject_permissions(&mut plan, &perms, Some(&params)).unwrap();

        match plan {
            OperatorPlan::Filter { predicate, .. } => match predicate {
                Predicate::And { predicates } => assert_eq!(predicates.len(), 2),
                other => panic!("expected And, got {:?}", other),
            },
            _ => panic!("expected Filter"),
        }
    }

    #[test]
    fn subquery_inner_scan_is_injected() {
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
        let perms = perms_with(&[("thread", "true"), ("comment", "author.id = $auth.id")]);
        let params = json!({"auth": {"id": "user:a"}});
        inject_permissions(&mut plan, &perms, Some(&params)).unwrap();

        if let OperatorPlan::Project { projections, .. } = &plan {
            if let Projection::Subquery { plan, .. } = &projections[0] {
                assert!(matches!(plan.as_ref(), OperatorPlan::Filter { .. }));
                return;
            }
        }
        panic!("expected Project with Subquery containing a Filter");
    }

    #[test]
    fn or_with_param_lhs_parses_through_shared_converter() {
        // Shape used by the example app's thread permission.
        let mut plan = OperatorPlan::Scan {
            table: "thread".into(),
        };
        let perms = perms_with(&[(
            "thread",
            "published = true OR $access = \"account\" AND author.id = $auth.id",
        )]);
        let params = json!({"auth": {"id": "user:a"}, "access": "account"});
        inject_permissions(&mut plan, &perms, Some(&params)).unwrap();

        match plan {
            OperatorPlan::Filter { predicate, .. } => {
                assert!(matches!(predicate, Predicate::Or { .. }));
            }
            _ => panic!("expected Filter"),
        }
    }

    /// Identifier-vs-identifier in a permission expression (e.g. `author = otherfield`)
    /// gets routed through the converter as a `__JOIN_CANDIDATE__` and lifted
    /// into a `Join` operator. We can't AND a Join into a scan filter, so the
    /// non-flat plan must surface as an error.
    #[test]
    fn join_candidate_permission_errors() {
        let mut plan = OperatorPlan::Scan {
            table: "thread".into(),
        };
        let perms = perms_with(&[("thread", "author = otherfield")]);
        let err = inject_permissions(&mut plan, &perms, None).unwrap_err();
        assert!(
            err.to_string().contains("non-filter") || err.to_string().contains("non-flat"),
            "expected join/non-filter error, got {err}"
        );
    }

    /// $auth check looks at a top-level `auth` field on the params object.
    /// `auth = null` must be treated as missing — null can't grant access.
    #[test]
    fn null_auth_param_is_treated_as_missing() {
        let mut plan = OperatorPlan::Scan {
            table: "thread".into(),
        };
        let perms = perms_with(&[("thread", "author.id = $auth.id")]);
        let params = json!({"auth": null});
        let err = inject_permissions(&mut plan, &perms, Some(&params)).unwrap_err();
        assert!(err.to_string().contains("$auth"));
    }

    /// A predicate that doesn't reference `$auth` at all should register fine
    /// even with no auth params (e.g. `published = true`).
    #[test]
    fn no_auth_required_works_without_params() {
        let mut plan = OperatorPlan::Scan {
            table: "thread".into(),
        };
        let perms = perms_with(&[("thread", "published = true")]);
        inject_permissions(&mut plan, &perms, None).unwrap();
        assert!(matches!(plan, OperatorPlan::Filter { .. }));
    }

    /// A join in the user query: each side scan must get its own permission
    /// applied independently. Verify by tracking both wrap-events.
    #[test]
    fn join_query_injects_both_sides() {
        use crate::operator::plan::JoinCondition;
        let mut plan = OperatorPlan::Join {
            left: Box::new(OperatorPlan::Scan {
                table: "thread".into(),
            }),
            right: Box::new(OperatorPlan::Scan {
                table: "user".into(),
            }),
            on: JoinCondition {
                left_field: Path::new("author"),
                right_field: Path::new("id"),
            },
        };
        let perms = perms_with(&[("thread", "true"), ("user", "id = $auth.id")]);
        let params = json!({"auth": {"id": "user:a"}});
        inject_permissions(&mut plan, &perms, Some(&params)).unwrap();

        // Left side `thread` is `true` so unchanged. Right side `user` must
        // now be wrapped in Filter.
        if let OperatorPlan::Join { right, .. } = &plan {
            assert!(
                matches!(right.as_ref(), OperatorPlan::Filter { .. }),
                "right side must be Filter-wrapped, got {right:?}"
            );
        } else {
            panic!("expected Join");
        }
    }

    /// Limit / Distinct wrappers around a scan must not block injection: the
    /// walker has to descend through them.
    #[test]
    fn injection_descends_through_limit_and_distinct() {
        let mut plan = OperatorPlan::Limit {
            input: Box::new(OperatorPlan::Distinct {
                input: Box::new(OperatorPlan::Scan {
                    table: "thread".into(),
                }),
            }),
            limit: 10,
            order_by: None,
        };
        let perms = perms_with(&[("thread", "published = true")]);
        inject_permissions(&mut plan, &perms, None).unwrap();

        // Walk down to find the wrapped scan.
        if let OperatorPlan::Limit { input, .. } = &plan {
            if let OperatorPlan::Distinct { input } = input.as_ref() {
                assert!(
                    matches!(input.as_ref(), OperatorPlan::Filter { .. }),
                    "scan must be wrapped"
                );
                return;
            }
        }
        panic!("walker did not descend through Limit/Distinct");
    }

    /// Permission text with leading/trailing whitespace must still parse —
    /// the boot loader trims when extracting, but a manual `set_permission`
    /// caller could pass whitespace.
    #[test]
    fn whitespace_around_permission_text_is_tolerated() {
        let mut plan = OperatorPlan::Scan {
            table: "thread".into(),
        };
        let perms = perms_with(&[("thread", "  published = true  ")]);
        inject_permissions(&mut plan, &perms, None).unwrap();
        assert!(matches!(plan, OperatorPlan::Filter { .. }));
    }

    /// AND-folding must short-circuit to `False` if either operand is False
    /// (e.g. the user predicate already evaluated to false statically).
    #[test]
    fn and_with_false_collapses_to_false() {
        let user_pred = Predicate::False;
        let mut plan = OperatorPlan::Filter {
            input: Box::new(OperatorPlan::Scan {
                table: "thread".into(),
            }),
            predicate: user_pred,
        };
        let perms = perms_with(&[("thread", "published = true")]);
        inject_permissions(&mut plan, &perms, None).unwrap();
        if let OperatorPlan::Filter { predicate, .. } = plan {
            assert!(matches!(predicate, Predicate::False));
        } else {
            panic!("expected Filter");
        }
    }

    /// Empty perms HashMap behaves the same as missing entries: every scan
    /// default-denies (except `_00_*`).
    #[test]
    fn empty_perms_map_default_denies_user_tables() {
        let mut plan = OperatorPlan::Scan {
            table: "user_table".into(),
        };
        let perms: HashMap<String, String> = HashMap::new();
        let err = inject_permissions(&mut plan, &perms, None).unwrap_err();
        assert!(err.to_string().contains("default-deny"));
    }

    /// Subquery whose plan is itself a Filter+Scan: the existing inner Filter
    /// must be ANDed, not double-wrapped (mirrors the top-level case).
    #[test]
    fn subquery_existing_filter_is_anded() {
        let inner_filter = OperatorPlan::Filter {
            input: Box::new(OperatorPlan::Scan {
                table: "comment".into(),
            }),
            predicate: Predicate::Eq {
                field: Path::new("active"),
                value: json!(true),
            },
        };
        let projection = Projection::Subquery {
            alias: "comments".into(),
            plan: Box::new(inner_filter),
            parent_key: None,
        };
        let mut plan = OperatorPlan::Project {
            input: Box::new(OperatorPlan::Scan {
                table: "thread".into(),
            }),
            projections: vec![projection],
        };
        let perms = perms_with(&[("thread", "true"), ("comment", "author.id = $auth.id")]);
        let params = json!({"auth": {"id": "user:a"}});
        inject_permissions(&mut plan, &perms, Some(&params)).unwrap();

        if let OperatorPlan::Project { projections, .. } = &plan {
            if let Projection::Subquery { plan, .. } = &projections[0] {
                if let OperatorPlan::Filter { predicate, .. } = plan.as_ref() {
                    match predicate {
                        Predicate::And { predicates } => {
                            assert_eq!(predicates.len(), 2);
                            return;
                        }
                        other => panic!("expected And, got {other:?}"),
                    }
                }
            }
        }
        panic!("did not find expected nested Filter");
    }
}
