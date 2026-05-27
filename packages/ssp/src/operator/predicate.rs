use crate::types::Path;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Filter predicates for query evaluation.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Predicate {
    /// Always true. Used for `WHERE true` permission rules.
    True,
    /// Always false. The fail-closed sentinel for unrepresentable permission
    /// expressions when `inject_permissions` cannot route them through the
    /// shared converter.
    False,
    Eq { field: Path, value: Value },
    Neq { field: Path, value: Value },
    Gt { field: Path, value: Value },
    Gte { field: Path, value: Value },
    Lt { field: Path, value: Value },
    Lte { field: Path, value: Value },
    Prefix { field: Path, prefix: String },
    /// `$param OP literal/param` — comparison with the parameter on the LHS.
    /// Resolved against the registration's params (`auth`, `access`, …) on each
    /// row, but the result is row-independent so the per-row evaluation is
    /// effectively a session-wide gate.
    ParamEq { param: String, value: Value },
    ParamNeq { param: String, value: Value },
    ParamGt { param: String, value: Value },
    ParamGte { param: String, value: Value },
    ParamLt { param: String, value: Value },
    ParamLte { param: String, value: Value },
    And { predicates: Vec<Predicate> },
    Or { predicates: Vec<Predicate> },
}
