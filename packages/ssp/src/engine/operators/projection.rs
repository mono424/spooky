use super::Operator;
use crate::engine::types::Path;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OrderSpec {
    pub field: Path,
    pub direction: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Projection {
    All,
    Field { name: Path },
    Subquery { alias: String, plan: Box<Operator> },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct JoinCondition {
    pub left_field: Path,
    pub right_field: Path,
}
