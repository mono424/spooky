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
    /// Semi-join: emit left keys with at least one right witness. Output
    /// weights are 0/1. Used by permission lowering for `IN (subquery)` and
    /// `EXISTS (subquery)` predicates.
    SemiJoin {
        left: Box<OperatorPlan>,
        right: Box<OperatorPlan>,
        on: JoinCondition,
    },
    /// Anti-join: emit left keys with no right witness. Output weights are
    /// 0/1. Used by permission lowering for `NOT IN (subquery)` and
    /// `NOT EXISTS (subquery)` predicates, and as the lowering of `Not(b)`.
    AntiJoin {
        left: Box<OperatorPlan>,
        right: Box<OperatorPlan>,
        on: JoinCondition,
    },
    /// Z-set additive merge. Used by permission lowering for `OR` of branches
    /// over the same outer scan, with a downstream `Distinct` collapsing
    /// duplicate keys to weight 1.
    Union {
        left: Box<OperatorPlan>,
        right: Box<OperatorPlan>,
    },
    /// Threshold to {0, 1} weights. Used to dedupe a `Union` of permission
    /// branches; reusable elsewhere too. The runtime operator already exists
    /// (`distinct.rs`); this variant just makes it reachable from a plan.
    Distinct {
        input: Box<OperatorPlan>,
    },
    Project {
        input: Box<OperatorPlan>,
        projections: Vec<Projection>,
    },
    Limit {
        input: Box<OperatorPlan>,
        limit: usize,
        /// Number of leading rows to skip (SurrealQL `START`). Defaults to 0,
        /// so plans emitted before offset support deserialize unchanged.
        #[serde(default)]
        start: usize,
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

/// Foreign key linking a subquery's child records to their parent.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SubqueryParentKey {
    /// Field on the child record that references the parent (e.g., "thread").
    pub child_field: String,
    /// Field on the parent record being referenced (e.g., "id").
    pub parent_field: String,
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
    Subquery {
        alias: String,
        plan: Box<OperatorPlan>,
        #[serde(default)]
        parent_key: Option<SubqueryParentKey>,
    },
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
            OperatorPlan::Filter { input, .. }
            | OperatorPlan::Limit { input, .. }
            | OperatorPlan::Distinct { input } => {
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
            OperatorPlan::Join { left, right, .. }
            | OperatorPlan::SemiJoin { left, right, .. }
            | OperatorPlan::AntiJoin { left, right, .. }
            | OperatorPlan::Union { left, right } => {
                left.collect_tables(tables);
                right.collect_tables(tables);
            }
        }
    }

    /// Every `(table, field_root)` pair this plan asks the circuit to EVALUATE:
    /// filter predicates, join keys, ORDER BY keys, and subquery parent keys.
    ///
    /// Projections are deliberately excluded. A projected field is passed
    /// through untouched (`operator/map.rs` ignores the projection list), so
    /// selecting a field the circuit does not hold is harmless — the client reads
    /// the value from SurrealDB, not from here. Evaluating one is not harmless:
    /// `eval::value_ops::resolve_field` returns `None` for an absent key, which
    /// silently makes the comparison false and drops the row from membership.
    /// That is what this list exists to reject at registration time.
    ///
    /// The table attributed to a field is the first table scanned in the subtree
    /// the field belongs to — exact for the single-scan case, and for joins each
    /// side is walked with its own table.
    pub fn evaluated_field_refs(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        self.collect_evaluated_field_refs(&mut out);
        let mut seen = std::collections::HashSet::new();
        out.retain(|pair| seen.insert(pair.clone()));
        out
    }

    fn collect_evaluated_field_refs(&self, out: &mut Vec<(String, String)>) {
        // Table a field mentioned in THIS node's clauses belongs to.
        let owner = |plan: &OperatorPlan| plan.referenced_tables().first().cloned();

        match self {
            OperatorPlan::Scan { .. } => {}
            OperatorPlan::Filter { input, predicate } => {
                if let Some(table) = owner(input) {
                    for field in predicate.field_roots() {
                        out.push((table.clone(), field));
                    }
                }
                input.collect_evaluated_field_refs(out);
            }
            OperatorPlan::Limit {
                input, order_by, ..
            } => {
                if let (Some(table), Some(specs)) = (owner(input), order_by.as_ref()) {
                    for spec in specs {
                        if let Some(root) = spec.field.segments().first() {
                            out.push((table.clone(), root.to_string()));
                        }
                    }
                }
                input.collect_evaluated_field_refs(out);
            }
            OperatorPlan::Distinct { input } => input.collect_evaluated_field_refs(out),
            OperatorPlan::Project { input, projections } => {
                input.collect_evaluated_field_refs(out);
                for proj in projections {
                    if let Projection::Subquery {
                        plan, parent_key, ..
                    } = proj
                    {
                        // The child key is read off the child row and the parent
                        // key off the parent row, so they belong to different
                        // tables. Both are evaluated to build `_00_list_ref`
                        // edges.
                        if let Some(key) = parent_key {
                            if let Some(child_table) = owner(plan) {
                                out.push((child_table, key.child_field.clone()));
                            }
                            if let Some(parent_table) = owner(input) {
                                out.push((parent_table, key.parent_field.clone()));
                            }
                        }
                        plan.collect_evaluated_field_refs(out);
                    }
                }
            }
            OperatorPlan::Join { left, right, on }
            | OperatorPlan::SemiJoin { left, right, on }
            | OperatorPlan::AntiJoin { left, right, on } => {
                if let (Some(lt), Some(root)) = (owner(left), on.left_field.segments().first()) {
                    out.push((lt, root.to_string()));
                }
                if let (Some(rt), Some(root)) = (owner(right), on.right_field.segments().first()) {
                    out.push((rt, root.to_string()));
                }
                left.collect_evaluated_field_refs(out);
                right.collect_evaluated_field_refs(out);
            }
            OperatorPlan::Union { left, right } => {
                left.collect_evaluated_field_refs(out);
                right.collect_evaluated_field_refs(out);
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

    /// Get metadata about subquery projections: (alias, table_name, parent_key, parent_table).
    /// `parent_table` is `None` for root-level subqueries (parent is in view.cache)
    /// and `Some(table)` for nested subqueries (parent is itself a subquery item).
    pub fn subquery_projection_info(&self) -> Vec<(String, String, Option<SubqueryParentKey>, Option<String>)> {
        let mut result = Vec::new();
        self.collect_subquery_projection_info(&mut result, None);
        result
    }

    fn collect_subquery_projection_info(
        &self,
        result: &mut Vec<(String, String, Option<SubqueryParentKey>, Option<String>)>,
        parent_table: Option<String>,
    ) {
        match self {
            OperatorPlan::Scan { .. } => {}
            OperatorPlan::Filter { input, .. }
            | OperatorPlan::Limit { input, .. }
            | OperatorPlan::Distinct { input } => {
                input.collect_subquery_projection_info(result, parent_table);
            }
            OperatorPlan::Project { input, projections } => {
                input.collect_subquery_projection_info(result, parent_table.clone());
                for proj in projections {
                    if let Projection::Subquery {
                        alias,
                        plan,
                        parent_key,
                    } = proj
                    {
                        let tables = plan.referenced_tables();
                        if let Some(table) = tables.first() {
                            result.push((
                                alias.clone(),
                                table.clone(),
                                parent_key.clone(),
                                parent_table.clone(),
                            ));
                            // Recurse into nested subquery plan
                            plan.collect_subquery_projection_info(result, Some(table.clone()));
                        }
                    }
                }
            }
            OperatorPlan::Join { left, right, .. }
            | OperatorPlan::SemiJoin { left, right, .. }
            | OperatorPlan::AntiJoin { left, right, .. }
            | OperatorPlan::Union { left, right } => {
                left.collect_subquery_projection_info(result, parent_table.clone());
                right.collect_subquery_projection_info(result, parent_table);
            }
        }
    }

    fn collect_subquery_tables(&self, tables: &mut Vec<String>) {
        match self {
            OperatorPlan::Scan { .. } => {}
            OperatorPlan::Filter { input, .. }
            | OperatorPlan::Limit { input, .. }
            | OperatorPlan::Distinct { input } => {
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
            OperatorPlan::Join { left, right, .. }
            | OperatorPlan::SemiJoin { left, right, .. }
            | OperatorPlan::AntiJoin { left, right, .. }
            | OperatorPlan::Union { left, right } => {
                left.collect_subquery_tables(tables);
                right.collect_subquery_tables(tables);
            }
        }
    }
}
