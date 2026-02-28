use crate::algebra::ZSet;
use crate::circuit::store::Store;
use crate::eval::value_ops::{compare_values, resolve_field};
use crate::operator::predicate::Predicate;
use crate::types::{Path, SpookyValue};
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::HashMap;

/// Filter operator: selects records matching a predicate.
///
/// Stateless (arity 1). The delta rule is identical to the snapshot rule:
/// `delta_out = filter(delta_in, predicate)`
///
/// This works because filter is a linear operator in DBSP:
/// `filter(A + B) = filter(A) + filter(B)`
#[derive(Debug)]
pub struct Filter {
    pub predicate: Predicate,
}

impl Filter {
    pub fn new(predicate: Predicate) -> Self {
        Self { predicate }
    }

    fn check_predicate(&self, key: &str, store: &Store, ctx: Option<&SpookyValue>) -> bool {
        check_predicate_recursive(&self.predicate, key, store, ctx)
    }
}

impl super::Operator for Filter {
    fn snapshot(&self, inputs: &[&ZSet], store: &Store, ctx: Option<&SpookyValue>) -> ZSet {
        let upstream = inputs[0];
        let mut out = HashMap::new();
        for (key, &weight) in upstream.iter() {
            if self.check_predicate(key, store, ctx) {
                out.insert(key.clone(), weight);
            }
        }
        out
    }

    fn step(
        &mut self,
        input_deltas: &[&ZSet],
        store: &Store,
        ctx: Option<&SpookyValue>,
    ) -> ZSet {
        // Filter is stateless: delta rule = apply predicate to delta
        self.snapshot(input_deltas, store, ctx)
    }

    fn arity(&self) -> usize {
        1
    }

    fn reset(&mut self) {}
}

/// Resolve a predicate value, handling $param references.
fn resolve_predicate_value(value: &Value, ctx: Option<&SpookyValue>) -> Option<SpookyValue> {
    if let Some(obj) = value.as_object() {
        if let Some(param_path) = obj.get("$param") {
            let ctx = ctx?;
            let path_str = param_path.as_str().unwrap_or("");
            let effective_path = if let Some(rest) = path_str.strip_prefix("parent.") {
                rest
            } else {
                path_str
            };
            let path = Path::new(effective_path);
            resolve_field(Some(ctx), &path).cloned()
        } else {
            Some(SpookyValue::from(value.clone()))
        }
    } else {
        Some(SpookyValue::from(value.clone()))
    }
}

fn check_predicate_recursive(
    pred: &Predicate,
    key: &str,
    store: &Store,
    ctx: Option<&SpookyValue>,
) -> bool {
    match pred {
        Predicate::And { predicates } => predicates
            .iter()
            .all(|p| check_predicate_recursive(p, key, store, ctx)),
        Predicate::Or { predicates } => predicates
            .iter()
            .any(|p| check_predicate_recursive(p, key, store, ctx)),
        Predicate::Prefix { field, prefix } => {
            if field.segments().len() == 1 && field.segments()[0] == "id" {
                return key.starts_with(prefix.as_str());
            }
            if let Some(row) = store.get_row_by_key(key) {
                if let Some(val) = resolve_field(Some(row), field) {
                    if let SpookyValue::Str(s) = val {
                        return s.starts_with(prefix.as_str());
                    }
                }
            }
            false
        }
        Predicate::Eq { field, value }
        | Predicate::Neq { field, value }
        | Predicate::Gt { field, value }
        | Predicate::Gte { field, value }
        | Predicate::Lt { field, value }
        | Predicate::Lte { field, value } => {
            let target = match resolve_predicate_value(value, ctx) {
                Some(v) => v,
                None => return false,
            };

            let actual = store
                .get_row_by_key(key)
                .and_then(|r| resolve_field(Some(r), field).cloned());

            if let Some(actual) = actual {
                let ord = compare_values(Some(&actual), Some(&target));
                match pred {
                    Predicate::Eq { .. } => ord == Ordering::Equal,
                    Predicate::Neq { .. } => ord != Ordering::Equal,
                    Predicate::Gt { .. } => ord == Ordering::Greater,
                    Predicate::Gte { .. } => ord != Ordering::Less,
                    Predicate::Lt { .. } => ord == Ordering::Less,
                    Predicate::Lte { .. } => ord != Ordering::Greater,
                    _ => false,
                }
            } else {
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operator::Operator;
    use crate::circuit::store::Change;
    use serde_json::json;

    fn zset(items: &[(&str, i64)]) -> ZSet {
        items.iter().map(|(k, w)| (k.to_string(), *w)).collect()
    }

    #[test]
    fn snapshot_filters_matching_records() {
        let mut store = Store::new();
        store.ensure_collection("users");
        store.apply_change(&Change::create("users", "user:1", json!({"level": 10})));
        store.apply_change(&Change::create("users", "user:2", json!({"level": 3})));

        let pred = Predicate::Gte {
            field: Path::new("level"),
            value: json!(5),
        };
        let filter = Filter::new(pred);
        let input = zset(&[("users:1", 1), ("users:2", 1)]);

        let result = filter.snapshot(&[&input], &store, None);
        assert_eq!(result.get("users:1"), Some(&1));
        assert!(!result.contains_key("users:2"));
    }

    #[test]
    fn step_is_identical_to_snapshot() {
        let mut store = Store::new();
        store.ensure_collection("users");
        store.apply_change(&Change::create("users", "user:1", json!({"level": 10})));
        store.apply_change(&Change::create("users", "user:2", json!({"level": 3})));

        let pred = Predicate::Gte {
            field: Path::new("level"),
            value: json!(5),
        };
        let delta = zset(&[("users:1", 1), ("users:2", 1)]);

        let snap = Filter::new(pred.clone()).snapshot(&[&delta], &store, None);
        let incr = Filter::new(pred).step(&[&delta], &store, None);
        assert_eq!(snap, incr);
    }

    #[test]
    fn step_preserves_negative_weights() {
        let mut store = Store::new();
        store.ensure_collection("users");
        store.apply_change(&Change::create("users", "user:1", json!({"level": 10})));

        let pred = Predicate::Gte {
            field: Path::new("level"),
            value: json!(5),
        };
        let mut filter = Filter::new(pred);
        let delta = zset(&[("users:1", -1)]);

        let result = filter.step(&[&delta], &store, None);
        assert_eq!(result.get("users:1"), Some(&-1));
    }
}
