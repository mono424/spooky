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
    /// Extract all table names referenced by this operator tree (deduplicated, order preserved)
    pub fn referenced_tables(&self) -> Vec<String> {
        let mut tables = Vec::new();
        self.collect_referenced_tables_recursive(&mut tables);
        
        // Deduplicate while preserving first-occurrence order
        let mut seen = std::collections::HashSet::new();
        tables.retain(|t| seen.insert(t.clone()));
        
        tables
    }
    
    /// Internal recursive collector - does NOT deduplicate
    fn collect_referenced_tables_recursive(&self, tables: &mut Vec<String>) {
        match self {
            Operator::Scan { table } => {
                tables.push(table.clone());
            }
            Operator::Filter { input, .. } | Operator::Limit { input, .. } => {
                input.collect_referenced_tables_recursive(tables);
            }
            Operator::Project { input, projections } => {
                input.collect_referenced_tables_recursive(tables);
                for proj in projections {
                    if let Projection::Subquery { plan, .. } = proj {
                        plan.collect_referenced_tables_recursive(tables);
                    }
                }
            }
            Operator::Join { left, right, .. } => {
                left.collect_referenced_tables_recursive(tables);
                right.collect_referenced_tables_recursive(tables);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::types::Path;

    #[test]
    fn test_referenced_tables_no_duplicates() {
        // Build a query plan that references "user" twice:
        // root -> Project -> (Scan thread, Subquery -> user, Subquery -> (Scan comment -> Subquery -> user))
        
        let user_scan = Operator::Scan { table: "user".to_string() };
        
        let comment_scan = Operator::Scan { table: "comment".to_string() };
        let comment_project = Operator::Project {
            input: Box::new(comment_scan),
            projections: vec![
                Projection::Subquery {
                    alias: "author".to_string(),
                    plan: Box::new(user_scan.clone()),
                }
            ],
        };

        let root = Operator::Project {
            input: Box::new(Operator::Scan { table: "thread".to_string() }),
            projections: vec![
                Projection::Subquery {
                    alias: "author".to_string(),
                    plan: Box::new(user_scan),
                },
                Projection::Subquery {
                    alias: "comments".to_string(),
                    plan: Box::new(comment_project),
                }
            ],
        };
        
        let tables = root.referenced_tables();
        
        // Should be deduplicated
        assert_eq!(tables.len(), 3, "Should have exactly 3 unique tables");
        // Verify order (thread discovered first, then user, then comment)
        assert_eq!(tables, vec!["thread", "user", "comment"]);
    }
    
    #[test]
    fn test_referenced_tables_simple_scan() {
        let op = Operator::Scan { table: "user".to_string() };
        let tables = op.referenced_tables();
        
        assert_eq!(tables, vec!["user"]);
    }
    
    #[test]
    fn test_referenced_tables_join() {
        let op = Operator::Join {
            left: Box::new(Operator::Scan { table: "user".to_string() }),
            right: Box::new(Operator::Scan { table: "user".to_string() }), // same table!
            on: JoinCondition {
                left_field: Path::new("id"),
                right_field: Path::new("user_id"),
            },
        };
        
        let tables = op.referenced_tables();
        
        // Should deduplicate even for self-join
        assert_eq!(tables, vec!["user"]);
    }
}
