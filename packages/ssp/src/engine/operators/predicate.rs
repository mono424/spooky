use crate::engine::circuit::Database;
use crate::engine::eval::{compare_spooky_values, normalize_record_id, resolve_nested_value};
use crate::engine::types::Path;
use crate::engine::types::SpookyValue;
use crate::engine::view::View;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Predicate {
    Prefix { field: Path, prefix: String },
    Eq { field: Path, value: Value },
    Neq { field: Path, value: Value },
    Gt { field: Path, value: Value },
    Gte { field: Path, value: Value },
    Lt { field: Path, value: Value },
    Lte { field: Path, value: Value },
    And { predicates: Vec<Predicate> },
    Or { predicates: Vec<Predicate> },
}

/// Resolve predicate value, handling $param references to context
fn resolve_predicate_value(value: &Value, context: Option<&SpookyValue>) -> Option<SpookyValue> {
    if let Some(obj) = value.as_object() {
        if let Some(param_path) = obj.get("$param") {
            let ctx = context?;
            let path_str = param_path.as_str().unwrap_or("");
            let effective_path = if path_str.starts_with("parent.") {
                &path_str[7..] // Strip "parent." prefix
            } else {
                path_str
            };
            let path = Path::new(effective_path);
            resolve_nested_value(Some(ctx), &path)
                .cloned()
                .map(normalize_record_id)
        } else {
            Some(SpookyValue::from(value.clone()))
        }
    } else {
        Some(SpookyValue::from(value.clone()))
    }
}

pub fn check_predicate(
    view: &View,
    pred: &Predicate,
    key: &str,
    db: &Database,
    context: Option<&SpookyValue>,
) -> bool {
    // Helper to get actual SpookyValue for comparison from the Predicate (which stores Value)

    match pred {
        Predicate::And { predicates } => predicates
            .iter()
            .all(|p| check_predicate(view, p, key, db, context)),
        Predicate::Or { predicates } => predicates
            .iter()
            .any(|p| check_predicate(view, p, key, db, context)),
        Predicate::Prefix { field, prefix } => {
            // Check if field value starts with prefix
            if field.0.len() == 1 && field.0[0] == "id" {
                return key.starts_with(prefix);
            }
            if let Some(row_val) = view.get_row_value(key, db) {
                if let Some(val) = resolve_nested_value(Some(row_val), field) {
                    if let SpookyValue::Str(s) = val {
                        return s.starts_with(prefix);
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
            let target_val = resolve_predicate_value(value, context);
            if target_val.is_none() {
                return false;
            }
            let target_val = target_val.unwrap();

            // FIX: Look up actual value from row even for "id", to ensure we match
            // the canonical ID stored in the DB (which might be "table:id").
            // The previous optimization incorrectly assumed stripped ID == Row ID.
            let actual_val_opt = view
                .get_row_value(key, db)
                .and_then(|r| resolve_nested_value(Some(r), field).cloned());

            if let Some(actual_val) = actual_val_opt {
                let ord = compare_spooky_values(Some(&actual_val), Some(&target_val));
                match pred {
                    Predicate::Eq { .. } => ord == Ordering::Equal,
                    Predicate::Neq { .. } => ord != Ordering::Equal,
                    Predicate::Gt { .. } => ord == Ordering::Greater,
                    Predicate::Gte { .. } => ord == Ordering::Greater || ord == Ordering::Equal,
                    Predicate::Lt { .. } => ord == Ordering::Less,
                    Predicate::Lte { .. } => ord == Ordering::Less || ord == Ordering::Equal,
                    _ => false,
                }
            } else {
                false
            }
        }
    }
}
