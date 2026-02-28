use crate::types::Path;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Filter predicates for query evaluation.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Predicate {
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
