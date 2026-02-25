use crate::types::Path;
use serde::{Deserialize, Serialize};

use super::predicate::Predicate;

/// A query plan is a tree of operator descriptions.
///
/// This is the deserialization format produced by the converter
/// (SurrealQL parser). It describes the logical query structure
/// but does NOT hold runtime state — that lives in the trait objects.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct QueryPlan {
    pub id: String,
    pub root: OperatorPlan,
}

/// Serializable operator description (the "plan" form).
///
/// This mirrors the old `Operator` enum and is used for:
/// - Deserialization from the converter
/// - Serialization for persistence
/// - Building the circuit Graph via `Graph::from_plan()`
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum OperatorPlan {
    Scan {
        table: String,
    },
    Filter {
        input: Box<OperatorPlan>,
        predicate: Predicate,
    },
    Join {
        left: Box<OperatorPlan>,
        right: Box<OperatorPlan>,
        on: JoinCondition,
    },
    Project {
        input: Box<OperatorPlan>,
        projections: Vec<Projection>,
    },
    Limit {
        input: Box<OperatorPlan>,
        limit: usize,
        #[serde(default)]
        order_by: Option<Vec<OrderSpec>>,
    },
}

/// Condition for equi-joins.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct JoinCondition {
    pub left_field: Path,
    pub right_field: Path,
}

/// Sort specification for ORDER BY.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OrderSpec {
    pub field: Path,
    pub direction: String,
}

/// Projection specification.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Projection {
    /// Select all fields (SELECT *).
    All,
    Field {
        #[serde(alias = "name")]
        field: Path,
        #[serde(default)]
        alias: Option<String>,
    },
    Subquery { alias: String, plan: Box<OperatorPlan> },
}

impl OperatorPlan {
    /// Collect all referenced table names (deduplicated, order preserved).
    pub fn referenced_tables(&self) -> Vec<String> {
        let mut tables = Vec::new();
        self.collect_tables(&mut tables);
        let mut seen = std::collections::HashSet::new();
        tables.retain(|t| seen.insert(t.clone()));
        tables
    }

    fn collect_tables(&self, tables: &mut Vec<String>) {
        match self {
            OperatorPlan::Scan { table } => tables.push(table.clone()),
            OperatorPlan::Filter { input, .. } | OperatorPlan::Limit { input, .. } => {
                input.collect_tables(tables);
            }
            OperatorPlan::Project { input, projections } => {
                input.collect_tables(tables);
                for proj in projections {
                    match proj {
                        Projection::Subquery { plan, .. } => plan.collect_tables(tables),
                        Projection::All | Projection::Field { .. } => {}
                    }
                }
            }
            OperatorPlan::Join { left, right, .. } => {
                left.collect_tables(tables);
                right.collect_tables(tables);
            }
        }
    }

    /// Collect table names referenced only inside `Projection::Subquery` plans.
    /// This set may overlap with primary (main-pipeline) tables.
    pub fn subquery_tables(&self) -> Vec<String> {
        let mut tables = Vec::new();
        self.collect_subquery_tables(&mut tables);
        let mut seen = std::collections::HashSet::new();
        tables.retain(|t| seen.insert(t.clone()));
        tables
    }

    fn collect_subquery_tables(&self, tables: &mut Vec<String>) {
        match self {
            OperatorPlan::Scan { .. } => {}
            OperatorPlan::Filter { input, .. } | OperatorPlan::Limit { input, .. } => {
                input.collect_subquery_tables(tables);
            }
            OperatorPlan::Project { input, projections } => {
                input.collect_subquery_tables(tables);
                for proj in projections {
                    if let Projection::Subquery { plan, .. } = proj {
                        // Collect ALL tables referenced within the subquery plan
                        plan.collect_tables(tables);
                    }
                }
            }
            OperatorPlan::Join { left, right, .. } => {
                left.collect_subquery_tables(tables);
                right.collect_subquery_tables(tables);
            }
        }
    }
}
