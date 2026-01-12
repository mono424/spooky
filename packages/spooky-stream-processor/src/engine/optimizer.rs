use crate::engine::view::{Operator, QueryPlan, Predicate, Path};

pub fn optimize(plan: QueryPlan) -> QueryPlan {
    let root = optimize_operator(plan.root);
    QueryPlan {
        id: plan.id,
        root,
    }
}

fn optimize_operator(op: Operator) -> Operator {
    match op {
        Operator::Scan { .. } => op, // Leaf
        Operator::Filter { input, predicate } => {
            // First optimize the input recursively
            let input = optimize_operator(*input);
            push_down_filter(input, predicate)
        }
        Operator::Project { input, projections } => {
            Operator::Project {
                input: Box::new(optimize_operator(*input)),
                projections,
            }
        }
        Operator::Limit { input, limit, order_by } => {
            Operator::Limit {
                input: Box::new(optimize_operator(*input)),
                limit,
                order_by
            }
        }
        Operator::Join { left, right, on } => {
            Operator::Join {
                left: Box::new(optimize_operator(*left)),
                right: Box::new(optimize_operator(*right)),
                on
            }
        }
    }
}

/// Tries to push the predicate down.
/// If success, returns the new subtree.
/// If fail (cannot push), returns Filter(input, predicate).
fn push_down_filter(input: Operator, predicate: Predicate) -> Operator {
    match input {
        Operator::Join { left, right, on } => {
            // Check if predicate refers ONLY to left fields or ONLY to right fields?
            // "Constraint: If the filter only applies to the Left table, move the Filter inside the Left branch"
            
            // To do this strictly, we need to know schema or check field prefixes in the predicate.
            // Assumption: Left fields act like "t1.foo" and Right like "t2.bar"?
            // Or if field paths match the Join condition?
            // Simplified logic: Check if all fields in predicate exist in Left part of ON condition?
            // Actually, we usually inspect the `Path` in the predicate.
            
            // Let's rely on a heuristic:
            // If predicate fields are not involved in right side? Or simply if they seem to belong to Left.
            // For now, let's implement the recursive structure but assume we can't determine "applies to Left" without schema.
            // Wait, the prompt said: "If a Filter is above a Join, and the filter only applies to the Left table..."
            
            // Implementation: Check if predicate checks fields that are available in left.
            // Since we lack schema, we can check if the predicate uses ANY field that creates a dependency?
            // Let's implement a simplified check: 
            // If fields start with "left." or "right."? No, usually it's "users.id".
            // We need to inspect `extract_tables` from `left` operator.
            let left_tables = extract_tables_from_op(&left);
            // If predicate refers to fields in `left_tables`, push left.
            
            // BUT predicate paths are like "age". We don't know which table "age" comes from if ambiguous.
            // However, typical SQL queries qualify: "users.age".
            // If predicate field matches table name in left?
            
            // Let's defer strict check and only push if we can PROVE it belongs to left.
            // Proof: Field path starts with table name found in left.
            
            if applies_to_any(&predicate, &left_tables) {
                // Push Left!
                Operator::Join {
                    left: Box::new(push_down_filter(*left, predicate)),
                    right,
                    on
                }
            } else {
                 // Keep it here
                 Operator::Filter {
                     input: Box::new(Operator::Join{ left, right, on }),
                     predicate
                 }
            }
        }
        _ => {
            // Default: Keep filter above
            Operator::Filter {
                input: Box::new(input),
                predicate
            }
        }
    }
}

fn extract_tables_from_op(op: &Operator) -> Vec<String> {
    match op {
        Operator::Scan { table } => vec![table.clone()],
        Operator::Filter { input, .. } => extract_tables_from_op(input),
        Operator::Project { input, .. } => extract_tables_from_op(input), // approximate
        Operator::Limit { input, .. } => extract_tables_from_op(input),
        Operator::Join { left, right, .. } => {
            let mut t = extract_tables_from_op(left);
            t.extend(extract_tables_from_op(right));
            t
        }
    }
}

fn applies_to_any(pred: &Predicate, tables: &[String]) -> bool {
    let fields = get_predicate_fields(pred);
    if fields.is_empty() { return false; } // Literal?
    
    // If ALL fields used in predicate start with one of the tables?
    // Example: field "users.age", tables ["users"] -> Match.
    // Example: field "age", tables ["users"] -> Ambiguous? Assume yes if single table?
    // Let's be strict: Must start with table name + dot.
    
    for field in fields {
        let s = field.as_str();
        let matches = tables.iter().any(|t| s.starts_with(t) && s.chars().nth(t.len()) == Some('.'));
        if !matches {
             return false;
        }
    }
    true
}

fn get_predicate_fields(pred: &Predicate) -> Vec<Path> {
    match pred {
        Predicate::Eq { field, .. } | Predicate::Neq { field, .. } |
        Predicate::Gt { field, .. } | Predicate::Gte { field, .. } |
        Predicate::Lt { field, .. } | Predicate::Lte { field, .. } |
        Predicate::Prefix { field, .. } => vec![field.clone()],
        Predicate::And { predicates } | Predicate::Or { predicates } => {
            predicates.iter().flat_map(get_predicate_fields).collect()
        }
    }
}
