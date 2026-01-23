use super::predicate::Predicate;
use super::projection::{JoinCondition, OrderSpec, Projection};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Operator {
    Scan {
        table: String,
    },
    Filter {
        input: Box<Operator>,
        predicate: Predicate,
    },
    Join {
        left: Box<Operator>,
        right: Box<Operator>,
        on: JoinCondition,
    },
    Project {
        input: Box<Operator>,
        projections: Vec<Projection>,
    },
    Limit {
        input: Box<Operator>,
        limit: usize,
        #[serde(default)]
        order_by: Option<Vec<OrderSpec>>,
    },
}

impl Operator {
    /// Extract all table names referenced by this operator tree
    pub fn referenced_tables(&self) -> Vec<String> {
        match self {
            Operator::Scan { table } => vec![table.clone()],
            Operator::Filter { input, .. } => input.referenced_tables(),
            Operator::Project { input, projections } => {
                let mut tables = input.referenced_tables();
                for p in projections {
                    if let Projection::Subquery { plan, .. } = p {
                        tables.extend(plan.referenced_tables());
                    }
                }
                tables
            }
            Operator::Limit { input, .. } => input.referenced_tables(),
            Operator::Join { left, right, .. } => {
                let mut tables = left.referenced_tables();
                tables.extend(right.referenced_tables());
                tables
            }
        }
    }

    /// Check if the operator tree contains any subquery projections
    pub fn has_subquery_projections(&self) -> bool {
        match self {
            Operator::Scan { .. } => false,
            Operator::Filter { input, .. } => input.has_subquery_projections(),
            Operator::Project { input, projections } => {
                let has_local_subquery = projections.iter().any(|p| matches!(p, Projection::Subquery { .. }));
                if has_local_subquery {
                    true
                } else {
                    input.has_subquery_projections()
                }
            },
            Operator::Limit { input, .. } => input.has_subquery_projections(),
            Operator::Join { left, right, .. } => {
                left.has_subquery_projections() || right.has_subquery_projections()
            }
        }
    }
}
