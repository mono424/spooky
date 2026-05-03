use crate::types::Path;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Filter predicates for query evaluation.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Predicate {
    /// Always true. Used for `WHERE true` permission rules.
    True,
    /// Always false. The fail-closed sentinel for permission expressions
    /// the SSP cannot translate (e.g. correlated subqueries).
    False,
    Eq { field: Path, value: Value },
    Neq { field: Path, value: Value },
    Gt { field: Path, value: Value },
    Gte { field: Path, value: Value },
    Lt { field: Path, value: Value },
    Lte { field: Path, value: Value },
    Prefix { field: Path, prefix: String },
    And { predicates: Vec<Predicate> },
    Or { predicates: Vec<Predicate> },
}

impl Predicate {
    /// Returns true if this predicate references any `$auth.*` parameter.
    /// Walks compound predicates recursively.
    pub fn references_auth(&self) -> bool {
        self.references_param("auth")
    }

    /// Returns true if this predicate references the given top-level $param namespace
    /// (e.g. "auth", "session"). Matches both `$auth` and `$auth.id`.
    pub fn references_param(&self, name: &str) -> bool {
        match self {
            Predicate::True | Predicate::False => false,
            Predicate::And { predicates } | Predicate::Or { predicates } => {
                predicates.iter().any(|p| p.references_param(name))
            }
            Predicate::Prefix { .. } => false,
            Predicate::Eq { value, .. }
            | Predicate::Neq { value, .. }
            | Predicate::Gt { value, .. }
            | Predicate::Gte { value, .. }
            | Predicate::Lt { value, .. }
            | Predicate::Lte { value, .. } => value_references_param(value, name),
        }
    }
}

fn value_references_param(value: &Value, name: &str) -> bool {
    if let Some(obj) = value.as_object() {
        if let Some(param) = obj.get("$param").and_then(|v| v.as_str()) {
            // Match `auth`, `auth.id`, `auth.anything` — but NOT a different
            // top-level name like `author.id`.
            return param == name || param.starts_with(&format!("{}.", name));
        }
    }
    false
}
