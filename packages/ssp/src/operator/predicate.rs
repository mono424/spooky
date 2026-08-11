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

impl Predicate {
    /// Root segment of every row field this predicate reads.
    ///
    /// `Param*` variants are excluded: they compare a registration parameter
    /// against a literal and never touch the row. Nested paths contribute only
    /// their head (`meta.secret` → `meta`), which is where a column-level marker
    /// lives.
    pub fn field_roots(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_field_roots(&mut out);
        out
    }

    fn collect_field_roots(&self, out: &mut Vec<String>) {
        let push = |out: &mut Vec<String>, path: &Path| {
            if let Some(root) = path.segments().first() {
                out.push(root.to_string());
            }
        };
        match self {
            Predicate::Eq { field, .. }
            | Predicate::Neq { field, .. }
            | Predicate::Gt { field, .. }
            | Predicate::Gte { field, .. }
            | Predicate::Lt { field, .. }
            | Predicate::Lte { field, .. }
            | Predicate::Prefix { field, .. } => push(out, field),
            Predicate::And { predicates } | Predicate::Or { predicates } => {
                for p in predicates {
                    p.collect_field_roots(out);
                }
            }
            Predicate::True
            | Predicate::False
            | Predicate::ParamEq { .. }
            | Predicate::ParamNeq { .. }
            | Predicate::ParamGt { .. }
            | Predicate::ParamGte { .. }
            | Predicate::ParamLt { .. }
            | Predicate::ParamLte { .. } => {}
        }
    }
}
