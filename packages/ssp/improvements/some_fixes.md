# Implementation Plan: Fix `referenced_tables` Duplicate Bug

## Problem Statement

The `referenced_tables()` method in `Operator` returns duplicate table names when the same table is referenced multiple times in nested subqueries.

**Example:** Query with thread author + comment authors both referencing `user`:
```rust
// Current output:
referenced_tables_cached: ["thread", "user", "comment", "user"]  // DUPLICATE!

// Expected output:
referenced_tables_cached: ["thread", "user", "comment"]  // DEDUPLICATED
```

**Impact:**
- `dependency_list` contains duplicate view indices: `"user": [0, 1, 2, 2]`
- Wasted memory and confusing debug output
- Potential for double-processing if dedup logic is ever removed

---

## Implementation Plan

### Phase 1: Fix `Operator::referenced_tables()` in `operators.rs`

**File:** `packages/ssp/src/engine/operators.rs`

**Current Implementation (assumed):**
```rust
impl Operator {
    pub fn referenced_tables(&self) -> Vec<String> {
        match self {
            Operator::Scan { table } => vec![table.clone()],
            Operator::Filter { input, .. } => input.referenced_tables(),
            Operator::Project { input, projections } => {
                let mut tables = input.referenced_tables();
                for proj in projections {
                    if let Projection::Subquery { plan, .. } = proj {
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
}
```

**New Implementation:**
```rust
impl Operator {
    /// Get all tables referenced by this operator tree (deduplicated, order preserved)
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
}
```

**Why this approach:**
- Single allocation for the Vec
- HashSet for O(1) duplicate detection
- `retain()` for in-place deduplication
- Preserves discovery order (thread before user before comment)

---

### Phase 2: Add Unit Test in `operators.rs`

**File:** `packages/ssp/src/engine/operators.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_referenced_tables_no_duplicates() {
        // Build a query plan that references "user" twice:
        // SELECT *, (author subquery), (comments with nested author subquery) FROM thread
        
        let user_subquery = Operator::Limit {
            input: Box::new(Operator::Filter {
                input: Box::new(Operator::Scan { table: "user".to_string() }),
                predicate: Predicate::Eq {
                    field: Path::new("id"),
                    value: serde_json::json!({"$param": "parent.author"}),
                },
            }),
            limit: 1,
            order_by: None,
        };
        
        let comments_subquery = Operator::Limit {
            input: Box::new(Operator::Project {
                input: Box::new(Operator::Filter {
                    input: Box::new(Operator::Scan { table: "comment".to_string() }),
                    predicate: Predicate::Eq {
                        field: Path::new("thread"),
                        value: serde_json::json!({"$param": "parent.id"}),
                    },
                }),
                projections: vec![
                    Projection::All,
                    Projection::Subquery {
                        alias: "author".to_string(),
                        plan: Box::new(user_subquery.clone()), // user referenced again!
                    },
                ],
            }),
            limit: 10,
            order_by: None,
        };
        
        let root = Operator::Limit {
            input: Box::new(Operator::Project {
                input: Box::new(Operator::Filter {
                    input: Box::new(Operator::Scan { table: "thread".to_string() }),
                    predicate: Predicate::Eq {
                        field: Path::new("id"),
                        value: serde_json::json!({"$param": "id"}),
                    },
                }),
                projections: vec![
                    Projection::All,
                    Projection::Subquery {
                        alias: "author".to_string(),
                        plan: Box::new(user_subquery), // user referenced first time
                    },
                    Projection::Subquery {
                        alias: "comments".to_string(),
                        plan: Box::new(comments_subquery), // contains nested user reference
                    },
                ],
            }),
            limit: 1,
            order_by: None,
        };
        
        let tables = root.referenced_tables();
        
        // Should be deduplicated
        assert_eq!(tables.len(), 3, "Should have exactly 3 unique tables");
        assert!(tables.contains(&"thread".to_string()));
        assert!(tables.contains(&"user".to_string()));
        assert!(tables.contains(&"comment".to_string()));
        
        // Verify no duplicates
        let unique: std::collections::HashSet<_> = tables.iter().collect();
        assert_eq!(unique.len(), tables.len(), "Should have no duplicates");
        
        // Verify order (thread discovered first, then user, then comment)
        assert_eq!(tables[0], "thread");
        assert_eq!(tables[1], "user");
        assert_eq!(tables[2], "comment");
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
            condition: JoinCondition::On {
                left_field: Path::new("id"),
                right_field: Path::new("user_id"),
            },
        };
        
        let tables = op.referenced_tables();
        
        // Should deduplicate even for self-join
        assert_eq!(tables, vec!["user"]);
    }
}
```

---

### Phase 3: Defensive Fix in `circuit.rs` (Belt and Suspenders)

Even though fixing `referenced_tables()` is the right solution, add defensive deduplication in the circuit for safety.

**File:** `packages/ssp/src/engine/circuit.rs`

**In `register_view()`:**
```rust
pub fn register_view(
    &mut self,
    plan: QueryPlan,
    params: Option<Value>,
    format: Option<ViewResultFormat>,
) -> Option<ViewUpdate> {
    // ... existing code ...
    
    self.views.push(view);
    let view_idx = self.views.len() - 1;

    // Get referenced tables (should already be deduplicated after Phase 1)
    let referenced = plan.root.referenced_tables();
    
    // DEFENSIVE: Log if we still see duplicates (indicates Phase 1 fix not applied)
    #[cfg(debug_assertions)]
    {
        let unique: std::collections::HashSet<_> = referenced.iter().collect();
        if unique.len() != referenced.len() {
            tracing::warn!(
                target: "ssp::circuit::register",
                view_id = %plan.id,
                referenced = ?referenced,
                "referenced_tables() returned duplicates - this should be fixed in Operator"
            );
        }
    }
    
    for t in referenced {
        self.dependency_list
            .entry(SmolStr::new(&t))
            .or_default()
            .push(view_idx);
    }
    
    // ... rest of code ...
}
```

**In `rebuild_dependency_list()`:**
```rust
pub fn rebuild_dependency_list(&mut self) {
    self.dependency_list.clear();
    for (i, view) in self.views.iter().enumerate() {
        // referenced_tables() should already be deduplicated after Phase 1
        for t in view.plan.root.referenced_tables() {
            self.dependency_list
                .entry(SmolStr::new(&t))
                .or_default()
                .push(i);
        }
    }
    
    // DEFENSIVE: Verify no duplicate view indices per table
    #[cfg(debug_assertions)]
    {
        for (table, indices) in &self.dependency_list {
            let unique: std::collections::HashSet<_> = indices.iter().collect();
            if unique.len() != indices.len() {
                tracing::error!(
                    target: "ssp::circuit",
                    table = %table,
                    indices = ?indices,
                    "Duplicate view indices in dependency_list!"
                );
            }
        }
    }
}
```

---

### Phase 4: Update `View::new()` and `initialize_after_deserialize()`

No changes needed - these already call `plan.root.referenced_tables()` which will now return deduplicated results.

---

## Testing Checklist

1. **Unit test:** `test_referenced_tables_no_duplicates` passes
2. **Unit test:** `test_referenced_tables_simple_scan` passes  
3. **Unit test:** `test_referenced_tables_join` passes
4. **Integration test:** Register the 3 views from your app and verify:
   - `referenced_tables_cached` has no duplicates
   - `dependency_list` has no duplicate view indices
5. **Manual test:** Debug output shows clean data

---

## Files to Modify

| File | Change |
|------|--------|
| `operators.rs` | Add `collect_referenced_tables_recursive()`, update `referenced_tables()` |
| `operators.rs` | Add unit tests |
| `circuit.rs` | Add defensive logging in `register_view()` and `rebuild_dependency_list()` |

---

## Estimated Effort

| Phase | Time |
|-------|------|
| Phase 1: Fix `referenced_tables()` | 15 min |
| Phase 2: Add unit tests | 20 min |
| Phase 3: Defensive logging | 10 min |
| Phase 4: Verification | 15 min |
| **Total** | **~1 hour** |

---

## Rollout

1. Apply fix to `operators.rs`
2. Run existing tests to ensure no regressions
3. Run new unit tests
4. Deploy and verify with real views
5. Check logs for any defensive warnings (should be none)