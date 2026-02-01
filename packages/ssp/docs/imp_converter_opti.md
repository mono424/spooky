# Implementation Plan: Query Plan Optimization for SSP

## Overview

This plan adds IVM-optimized query plan generation to your converter. We'll implement optimizations in phases, with each phase being independently testable and deployable.

**Timeline:** 2-3 weeks
**Difficulty:** Medium
**Files Modified:** Primarily `converter.rs`, minimal changes elsewhere

---

## Phase 1: Predicate Pushdown (Week 1, Days 1-3)

### Goal
Convert `filter(scan(table))` → `scan(table, filter)` to enable O(1) delta filtering instead of O(N).

### Benefits
- **100x faster** for queries with selective filters
- Single biggest IVM performance win
- Enables index usage in scan operator

---

### Step 1.1: Extend Scan Operator JSON Schema (1 hour)

**What to do:**
Your scan operator currently looks like:
```json
{"op": "scan", "table": "thread"}
```

It needs to support an optional filter:
```json
{"op": "scan", "table": "thread", "filter": {...}}
```

**Action items:**

1. **In your `Operator` enum** (wherever it's defined - likely in `engine/operators.rs` or similar):
   - Add an optional `filter` field to the `Scan` variant
   - Make sure it deserializes correctly from JSON

**Example change pattern:**
```rust
// Before
Scan {
    table: String,
}

// After
Scan {
    table: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter: Option<Predicate>,  // Or whatever your predicate type is
}
```

2. **Test deserialization:**
```rust
#[test]
fn test_scan_with_filter_deserializes() {
    let json = json!({
        "op": "scan",
        "table": "users",
        "filter": {
            "type": "eq",
            "field": "id",
            "value": "user:123"
        }
    });
    
    let op: Result<Operator, _> = serde_json::from_value(json);
    assert!(op.is_ok(), "Scan with filter should deserialize");
}
```

**Checkpoint:** Scan operator can deserialize with optional filter field.

---

### Step 1.2: Implement Filter Pushdown Logic in Converter (2 hours)

**File:** `converter.rs`

Add this new function right after your existing helper functions:

```rust
/// Pushes filter predicates down into scan operators where possible
fn optimize_predicate_pushdown(plan: Value) -> Value {
    // Pattern match on the plan structure
    if let Some(obj) = plan.as_object() {
        let op = obj.get("op").and_then(|v| v.as_str());
        
        // Pattern: filter → scan (this is what we want to optimize)
        if op == Some("filter") {
            if let Some(input) = obj.get("input") {
                if let Some(input_obj) = input.as_object() {
                    if input_obj.get("op").and_then(|v| v.as_str()) == Some("scan") {
                        // We found filter(scan(...))
                        let predicate = obj.get("predicate").unwrap();
                        
                        // Check if this predicate can be pushed down
                        if is_pushable_predicate(predicate) {
                            // Create new scan with filter embedded
                            let mut new_scan = input_obj.clone();
                            new_scan.insert("filter".to_string(), predicate.clone());
                            return Value::Object(new_scan);
                        }
                    }
                }
            }
        }
        
        // Recursively optimize children
        let mut optimized = obj.clone();
        for (key, value) in &mut optimized {
            if key == "input" || key == "left" || key == "right" {
                *value = optimize_predicate_pushdown(value.clone());
            }
        }
        return Value::Object(optimized);
    }
    
    plan
}

/// Checks if a predicate can be safely pushed into a scan
fn is_pushable_predicate(predicate: &Value) -> bool {
    let pred_type = predicate.get("type").and_then(|v| v.as_str());
    
    match pred_type {
        // Simple predicates on single fields can always be pushed
        Some("eq") | Some("gt") | Some("lt") | 
        Some("gte") | Some("lte") | Some("neq") | 
        Some("prefix") => true,
        
        // Logical operators: pushable if ALL children are pushable
        Some("and") | Some("or") => {
            predicate.get("predicates")
                .and_then(|v| v.as_array())
                .map(|preds| preds.iter().all(is_pushable_predicate))
                .unwrap_or(false)
        }
        
        // Everything else (joins, subqueries, etc.) cannot be pushed
        _ => false,
    }
}
```

**Integrate into conversion pipeline:**

Modify your `convert_surql_to_dbsp` function:

```rust
pub fn convert_surql_to_dbsp(sql: &str) -> Result<Value> {
    let clean_sql = sql.trim().trim_end_matches(';');
    match parse_full_query(clean_sql) {
        Ok((_, plan)) => {
            // NEW: Apply optimization before returning
            let optimized = optimize_predicate_pushdown(plan);
            Ok(optimized)
        }
        Err(e) => Err(anyhow!("SQL Parsing Error: {}", e)),
    }
}
```

**Checkpoint:** Converter outputs plans with filters pushed into scans.

---

### Step 1.3: Add Tests for Pushdown (1 hour)

Add these tests to the bottom of `converter.rs`:

```rust
#[cfg(test)]
mod pushdown_tests {
    use super::*;

    #[test]
    fn test_simple_filter_pushdown() {
        let sql = "SELECT * FROM users WHERE id = 'user:123'";
        let plan = convert_surql_to_dbsp(sql).unwrap();
        
        // Should produce scan with embedded filter, not filter(scan(...))
        assert_eq!(plan.get("op").and_then(|v| v.as_str()), Some("scan"));
        assert!(plan.get("filter").is_some(), "Filter should be pushed into scan");
    }

    #[test]
    fn test_multiple_filters_pushdown() {
        let sql = "SELECT * FROM users WHERE age > 18 AND active = true";
        let plan = convert_surql_to_dbsp(sql).unwrap();
        
        assert_eq!(plan.get("op").and_then(|v| v.as_str()), Some("scan"));
        
        // Should have AND filter pushed down
        let filter = plan.get("filter").unwrap();
        assert_eq!(filter.get("type").and_then(|v| v.as_str()), Some("and"));
    }

    #[test]
    fn test_join_not_pushed() {
        // Joins should NOT be pushed into scans
        let sql = "SELECT * FROM posts WHERE author_id = users.id";
        let plan = convert_surql_to_dbsp(sql).unwrap();
        
        // Should still be a join operator, not pushed into scan
        assert_eq!(plan.get("op").and_then(|v| v.as_str()), Some("join"));
    }
}
```

**Checkpoint:** All tests pass.

---

### Step 1.4: Update Scan Evaluation Logic (Location: TBD - you need to do this)

**What to do:**
In your scan operator evaluation function (likely in `view.rs` or `operators.rs`), add filter support.

**Instructions:**

1. **Find the function that evaluates Scan operators**
   - Search for: `Operator::Scan` or `fn eval_scan` or similar
   - It currently probably just returns all records from a table

2. **Add filter evaluation logic:**

**Pseudocode pattern:**
```rust
// Before (pseudocode)
fn eval_scan(table: &str, context: &Context) -> ZSet {
    // Get all records from table
    let records = context.get_table(table);
    records.clone()  // Return everything
}

// After (pseudocode)
fn eval_scan(table: &str, filter: Option<&Predicate>, context: &Context) -> ZSet {
    let all_records = context.get_table(table);
    
    // NEW: Apply filter if present
    if let Some(filter) = filter {
        all_records.iter()
            .filter(|(record, _weight)| eval_predicate(record, filter))
            .collect()
    } else {
        all_records.clone()
    }
}
```

3. **Important for IVM:** When processing deltas, the filter should be checked BEFORE adding to ZSet:

**Pseudocode pattern:**
```rust
// In delta processing (wherever that happens)
fn process_delta(delta: Record, operation: Operation) {
    match operator {
        Operator::Scan { table, filter } => {
            // NEW: Check filter before propagating delta
            if let Some(filter) = filter {
                if !eval_predicate(&delta, filter) {
                    // Delta doesn't match filter, don't propagate
                    return ZSet::empty();
                }
            }
            
            // Delta matches (or no filter), propagate it
            let mut result = ZSet::new();
            result.insert(delta, weight);
            result
        }
        // ... other operators
    }
}
```

**What I can't provide:** The exact code because I don't see your eval functions.

**What you need to do:** 
- Locate scan evaluation in your codebase
- Add filter parameter
- Apply filter during both full scan AND delta processing
- Test with the existing test suite

**Checkpoint:** Scans with filters only return matching records.

---

### Step 1.5: End-to-End Integration Test (1 hour)

**What to do:**
Create an integration test that proves the optimization works end-to-end.

**File:** Create new file `tests/query_optimization_test.rs` (or add to existing integration tests)

**Instructions:**

```rust
// Integration test pattern
#[test]
fn test_predicate_pushdown_performance() {
    // 1. Create circuit
    let mut circuit = Circuit::new();
    
    // 2. Register view with selective filter
    let sql = "SELECT * FROM users WHERE id = 'user:123'";
    let view_id = circuit.register_view(sql).unwrap();
    
    // 3. Insert 1000 users (only one matches)
    for i in 0..1000 {
        let user = json!({
            "id": format!("user:{}", i),
            "name": format!("User {}", i)
        });
        circuit.ingest("users", user, Operation::Create).unwrap();
    }
    
    // 4. Verify view only contains the ONE matching record
    let view = circuit.get_view(view_id).unwrap();
    assert_eq!(view.len(), 1);
    assert!(view.contains_key("user:123"));
    
    // 5. Benchmark: Update the matching user
    let start = std::time::Instant::now();
    circuit.ingest("users", json!({"id": "user:123", "name": "Updated"}), 
                   Operation::Update).unwrap();
    let duration = start.elapsed();
    
    // Should be <100μs with pushdown, >10ms without
    assert!(duration.as_micros() < 1000, 
            "Update took too long: {:?}", duration);
}
```

**Checkpoint:** Integration test passes, demonstrating performance improvement.

---

## Phase 2: Constant Folding & Simplification (Week 1, Days 4-5)

### Goal
Remove duplicate predicates and simplify boolean logic to reduce evaluation overhead.

### Benefits
- Cleaner query plans
- 10-20% faster predicate evaluation
- Easier debugging

---

### Step 2.1: Implement Predicate Simplification (2 hours)

**File:** `converter.rs`

Add this function after `is_pushable_predicate`:

```rust
/// Simplifies predicate logic by removing duplicates and flattening
fn simplify_predicate(pred: Value) -> Value {
    let pred_type = pred.get("type").and_then(|v| v.as_str());
    
    match pred_type {
        Some("and") | Some("or") => {
            // Get child predicates
            let preds = pred.get("predicates")
                .and_then(|v| v.as_array())
                .unwrap_or(&vec![]);
            
            // Recursively simplify each child
            let simplified: Vec<Value> = preds.iter()
                .map(|p| simplify_predicate(p.clone()))
                .collect();
            
            // Remove duplicates by converting to string (for comparison)
            let mut unique = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for p in simplified {
                let key = serde_json::to_string(&p).unwrap();
                if seen.insert(key) {
                    unique.push(p);
                }
            }
            
            // If only one predicate left, return it directly
            if unique.len() == 1 {
                return unique[0].clone();
            }
            
            // If no predicates, this shouldn't happen, but handle it
            if unique.is_empty() {
                return json!({"type": "always_true"});
            }
            
            // Return simplified AND/OR
            json!({
                "type": pred_type.unwrap(),
                "predicates": unique
            })
        }
        _ => pred,  // Other predicate types returned as-is
    }
}

/// Folds constant expressions (future enhancement)
fn constant_folding(plan: Value) -> Value {
    // For now, just return as-is
    // Future: Evaluate constant expressions like 10 + 8 -> 18
    plan
}
```

**Integrate into pipeline:**

Modify `convert_surql_to_dbsp`:

```rust
pub fn convert_surql_to_dbsp(sql: &str) -> Result<Value> {
    let clean_sql = sql.trim().trim_end_matches(';');
    match parse_full_query(clean_sql) {
        Ok((_, plan)) => {
            // Apply optimizations in order
            let mut optimized = plan;
            
            // Phase 1: Simplification
            optimized = simplify_plan(optimized);
            
            // Phase 2: Pushdown
            optimized = optimize_predicate_pushdown(optimized);
            
            Ok(optimized)
        }
        Err(e) => Err(anyhow!("SQL Parsing Error: {}", e)),
    }
}

/// Recursively simplify predicates throughout the plan
fn simplify_plan(plan: Value) -> Value {
    if let Some(mut obj) = plan.as_object().cloned() {
        // Simplify predicate if present
        if let Some(pred) = obj.get("predicate") {
            obj.insert("predicate".to_string(), simplify_predicate(pred.clone()));
        }
        
        // Recursively simplify children
        for (key, value) in &mut obj {
            if key == "input" || key == "left" || key == "right" {
                *value = simplify_plan(value.clone());
            }
        }
        
        Value::Object(obj)
    } else {
        plan
    }
}
```

---

### Step 2.2: Add Tests (1 hour)

```rust
#[test]
fn test_duplicate_predicate_removal() {
    // Manually construct a plan with duplicate predicates
    let plan = json!({
        "op": "filter",
        "predicate": {
            "type": "and",
            "predicates": [
                {"type": "eq", "field": "active", "value": true},
                {"type": "eq", "field": "active", "value": true},  // Duplicate!
            ]
        },
        "input": {"op": "scan", "table": "users"}
    });
    
    let simplified = simplify_plan(plan);
    
    // Should have only ONE predicate now
    let pred = simplified.get("predicate").unwrap();
    // After simplification, should be just the single predicate (not AND with one child)
    assert_eq!(pred.get("type").and_then(|v| v.as_str()), Some("eq"));
}

#[test]
fn test_nested_and_simplification() {
    let plan = json!({
        "op": "filter",
        "predicate": {
            "type": "and",
            "predicates": [
                {
                    "type": "and",
                    "predicates": [
                        {"type": "eq", "field": "a", "value": 1},
                        {"type": "eq", "field": "b", "value": 2},
                    ]
                },
                {"type": "eq", "field": "c", "value": 3},
            ]
        },
        "input": {"op": "scan", "table": "users"}
    });
    
    let simplified = simplify_plan(plan);
    
    // Should flatten nested ANDs
    let pred = simplified.get("predicate").unwrap();
    let predicates = pred.get("predicates").unwrap().as_array().unwrap();
    
    // Should have 3 predicates at top level (flattened)
    assert_eq!(predicates.len(), 3);
}
```

**Checkpoint:** Simplification tests pass.

---

## Phase 3: Projection Pruning (Week 2, Days 1-3)

### Goal
Only fetch fields that are actually used downstream, reducing memory usage.

### Benefits
- 20-50% memory savings
- Faster ZSet operations (fewer fields to compare)

---

### Step 3.1: Implement Field Usage Analysis (3 hours)

**File:** `converter.rs`

This is more complex - add these functions:

```rust
use std::collections::HashSet;

/// Analyzes which fields are actually used in a query plan
fn analyze_used_fields(plan: &Value) -> HashSet<String> {
    let mut fields = HashSet::new();
    
    let op = plan.get("op").and_then(|v| v.as_str());
    
    match op {
        Some("filter") => {
            // Extract fields from predicate
            extract_fields_from_predicate(
                plan.get("predicate").unwrap(),
                &mut fields
            );
            
            // Recurse on input
            if let Some(input) = plan.get("input") {
                fields.extend(analyze_used_fields(input));
            }
        }
        
        Some("project") => {
            // Only specific fields are needed
            if let Some(projs) = plan.get("projections").and_then(|v| v.as_array()) {
                for proj in projs {
                    if proj.get("type").and_then(|v| v.as_str()) == Some("field") {
                        if let Some(name) = proj.get("name").and_then(|v| v.as_str()) {
                            fields.insert(name.to_string());
                        }
                    } else if proj.get("type").and_then(|v| v.as_str()) == Some("all") {
                        // SELECT * means we need all fields
                        fields.insert("*".to_string());
                    }
                }
            }
            
            // Recurse on input
            if let Some(input) = plan.get("input") {
                fields.extend(analyze_used_fields(input));
            }
        }
        
        Some("join") => {
            // Need join key fields
            if let Some(on) = plan.get("on").and_then(|v| v.as_object()) {
                if let Some(left) = on.get("left_field").and_then(|v| v.as_str()) {
                    fields.insert(left.to_string());
                }
                if let Some(right) = on.get("right_field").and_then(|v| v.as_str()) {
                    fields.insert(right.to_string());
                }
            }
            
            // Recurse on both sides
            if let Some(left) = plan.get("left") {
                fields.extend(analyze_used_fields(left));
            }
            if let Some(right) = plan.get("right") {
                fields.extend(analyze_used_fields(right));
            }
        }
        
        Some("limit") | Some("scan") => {
            // Recurse on input if present
            if let Some(input) = plan.get("input") {
                fields.extend(analyze_used_fields(input));
            }
        }
        
        _ => {}
    }
    
    fields
}

/// Extracts field names from a predicate
fn extract_fields_from_predicate(pred: &Value, fields: &mut HashSet<String>) {
    let pred_type = pred.get("type").and_then(|v| v.as_str());
    
    match pred_type {
        Some("eq") | Some("gt") | Some("lt") | 
        Some("gte") | Some("lte") | Some("neq") | 
        Some("prefix") => {
            // Simple predicate - extract field name
            if let Some(field) = pred.get("field").and_then(|v| v.as_str()) {
                fields.insert(field.to_string());
            }
        }
        
        Some("and") | Some("or") => {
            // Logical operator - recurse on children
            if let Some(preds) = pred.get("predicates").and_then(|v| v.as_array()) {
                for p in preds {
                    extract_fields_from_predicate(p, fields);
                }
            }
        }
        
        _ => {}
    }
}

/// Adds projection pruning to scan operators
fn add_projection_pruning(plan: Value, needed_fields: &HashSet<String>) -> Value {
    if let Some(mut obj) = plan.as_object().cloned() {
        let op = obj.get("op").and_then(|v| v.as_str());
        
        if op == Some("scan") {
            // Add projection if we don't need all fields
            if !needed_fields.contains("*") && !needed_fields.is_empty() {
                // Convert HashSet to sorted Vec for determinism
                let mut field_vec: Vec<String> = needed_fields.iter().cloned().collect();
                field_vec.sort();
                
                obj.insert("projection".to_string(), json!(field_vec));
            }
        }
        
        // Recurse on children
        for (key, value) in &mut obj {
            if key == "input" || key == "left" || key == "right" {
                *value = add_projection_pruning(value.clone(), needed_fields);
            }
        }
        
        Value::Object(obj)
    } else {
        plan
    }
}
```

**Integrate into pipeline:**

```rust
pub fn convert_surql_to_dbsp(sql: &str) -> Result<Value> {
    let clean_sql = sql.trim().trim_end_matches(';');
    match parse_full_query(clean_sql) {
        Ok((_, plan)) => {
            let mut optimized = plan;
            
            // Phase 1: Simplification
            optimized = simplify_plan(optimized);
            
            // Phase 2: Pushdown
            optimized = optimize_predicate_pushdown(optimized);
            
            // Phase 3: Projection pruning
            let needed_fields = analyze_used_fields(&optimized);
            optimized = add_projection_pruning(optimized, &needed_fields);
            
            Ok(optimized)
        }
        Err(e) => Err(anyhow!("SQL Parsing Error: {}", e)),
    }
}
```

---

### Step 3.2: Update Scan Operator to Use Projection (Location: TBD)

**What to do:**

1. **Find scan evaluation function** (same place as Step 1.4)

2. **Add projection support:**

**Pseudocode pattern:**
```rust
fn eval_scan(
    table: &str, 
    filter: Option<&Predicate>,
    projection: Option<&Vec<String>>,  // NEW
    context: &Context
) -> ZSet {
    let all_records = context.get_table(table);
    
    // Apply filter
    let filtered = if let Some(filter) = filter {
        all_records.iter()
            .filter(|(record, _)| eval_predicate(record, filter))
            .collect()
    } else {
        all_records.clone()
    };
    
    // NEW: Apply projection
    if let Some(fields) = projection {
        filtered.into_iter()
            .map(|(record, weight)| {
                let projected = project_record(record, fields);
                (projected, weight)
            })
            .collect()
    } else {
        filtered
    }
}

// Helper to project a record to specific fields
fn project_record(record: &Record, fields: &[String]) -> Record {
    // Implementation depends on your Record type
    // Pseudocode:
    let mut projected = Record::new();
    for field in fields {
        if let Some(value) = record.get(field) {
            projected.insert(field.clone(), value.clone());
        }
    }
    projected
}
```

**What you need to do:**
- Locate scan evaluation
- Add projection parameter
- Implement field projection logic
- Test with integration tests

**Checkpoint:** Scans only return requested fields.

---

### Step 3.3: Tests (1 hour)

```rust
#[test]
fn test_projection_pruning_analysis() {
    let sql = "SELECT name FROM users WHERE age > 18";
    let plan = convert_surql_to_dbsp(sql).unwrap();
    
    // Scan should include projection limiting to needed fields
    let scan = find_scan_in_plan(&plan).expect("Should have scan");
    let projection = scan.get("projection").expect("Should have projection");
    
    let fields: Vec<String> = serde_json::from_value(projection.clone()).unwrap();
    
    // Should only include 'name' and 'age' (age needed for filter)
    assert!(fields.contains(&"name".to_string()));
    assert!(fields.contains(&"age".to_string()));
    assert_eq!(fields.len(), 2);
}

// Helper function
fn find_scan_in_plan(plan: &Value) -> Option<Value> {
    if plan.get("op").and_then(|v| v.as_str()) == Some("scan") {
        return Some(plan.clone());
    }
    
    // Recurse through plan structure
    for key in &["input", "left", "right"] {
        if let Some(child) = plan.get(key) {
            if let Some(scan) = find_scan_in_plan(child) {
                return Some(scan);
            }
        }
    }
    
    None
}
```

---

## Phase 4: Join Ordering (Week 2, Days 4-5) - OPTIONAL

### Goal
Order joins by estimated table size to minimize intermediate results.

### Benefits
- 2-10x faster for multi-join queries
- Significantly reduced memory usage

**Note:** This is OPTIONAL - only implement if you have queries with 2+ joins.

---

### Step 4.1: Add Table Statistics (2 hours)

**Instructions:**

You need to maintain statistics about table sizes. This can be done in several ways:

**Option A: Static configuration**
```rust
// In converter.rs or a new stats.rs file
lazy_static! {
    static ref TABLE_STATS: HashMap<&'static str, usize> = {
        let mut m = HashMap::new();
        m.insert("users", 10_000);
        m.insert("posts", 100_000);
        m.insert("comments", 500_000);
        m.insert("sessions", 1_000_000);
        // ... add your tables
        m
    };
}

fn estimate_table_size(table: &str) -> usize {
    TABLE_STATS.get(table).copied().unwrap_or(1_000)
}
```

**Option B: Dynamic from circuit** (better but requires circuit integration)
```rust
// You would need to pass table statistics from circuit to converter
// This is more complex and requires refactoring
```

**For now, use Option A** (static configuration).

---

### Step 4.2: Implement Join Ordering (2 hours)

**File:** `converter.rs`

Modify your `wrap_conditions` function to order joins:

```rust
fn wrap_conditions(input_op: Value, predicate: Value) -> Value {
    let mut joins = Vec::new();
    let mut filters = Vec::new();
    
    // ... existing partition logic ...
    
    let mut current_op = input_op;
    
    // NEW: Order joins by estimated size
    joins.sort_by_key(|join_pred| {
        let right_full = join_pred
            .get("right")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        
        let parts: Vec<&str> = right_full.split('.').collect();
        let table = if parts.len() > 1 { parts[0] } else { right_full };
        
        estimate_table_size(table)  // Smallest first
    });
    
    // Apply Joins in optimized order
    for join_pred in joins {
        // ... existing join application logic ...
    }
    
    // ... rest of function unchanged ...
}
```

---

### Step 4.3: Tests (1 hour)

```rust
#[test]
fn test_join_ordering() {
    // Query with multiple joins - larger tables should be joined last
    let sql = "SELECT * FROM posts 
               WHERE author_id = users.id 
               AND thread_id = threads.id";
    
    let plan = convert_surql_to_dbsp(sql).unwrap();
    
    // Walk the join tree and verify order
    // Smaller table (users) should be joined before larger (posts)
    // This test is complex - implement based on your needs
}
```

---

## Phase 5: Testing & Validation (Week 3)

### Step 5.1: Comprehensive Test Suite (2 days)

Create `tests/query_optimization_comprehensive.rs`:

```rust
use ssp::*;  // Adjust based on your crate structure

#[test]
fn test_optimization_correctness() {
    // Verify optimizations don't change query semantics
    let test_cases = vec![
        "SELECT * FROM users WHERE age > 18",
        "SELECT name FROM users WHERE active = true AND age > 18",
        "SELECT * FROM posts WHERE author_id = users.id",
        // ... add more
    ];
    
    for sql in test_cases {
        // Parse with optimizations
        let optimized = convert_surql_to_dbsp(sql).unwrap();
        
        // Verify it deserializes to valid operator
        let op: Operator = serde_json::from_value(optimized).unwrap();
        
        // Additional semantic checks...
    }
}

#[test]
fn test_optimization_performance() {
    // Create circuit with test data
    let mut circuit = Circuit::new();
    
    // Register optimized view
    let sql = "SELECT * FROM users WHERE id = 'user:123'";
    circuit.register_view(sql).unwrap();
    
    // Benchmark operations
    // ...
}
```

---

### Step 5.2: Performance Benchmarking (2 days)

**Instructions:**

1. **Create benchmark suite** in `benches/query_optimization.rs`:

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn benchmark_filter_pushdown(c: &mut Criterion) {
    c.bench_function("selective_filter_with_pushdown", |b| {
        let mut circuit = Circuit::new();
        let view_id = circuit.register_view(
            "SELECT * FROM users WHERE id = 'user:123'"
        ).unwrap();
        
        // Insert 10K users
        for i in 0..10_000 {
            circuit.ingest("users", 
                json!({"id": format!("user:{}", i)}),
                Operation::Create
            ).unwrap();
        }
        
        b.iter(|| {
            circuit.ingest("users",
                json!({"id": "user:123", "name": "Updated"}),
                Operation::Update
            ).unwrap();
        });
    });
}

criterion_group!(benches, benchmark_filter_pushdown);
criterion_main!(benches);
```

2. **Run benchmarks:**
```bash
cargo bench --bench query_optimization
```

3. **Compare before/after:**
   - Create a branch with optimizations
   - Benchmark
   - Compare with main branch
   - Document speedups

---

### Step 5.3: Documentation (1 day)

**Create documentation** explaining the optimizations:

**File:** `docs/QUERY_OPTIMIZATION.md`

```markdown
# Query Plan Optimizations

## Overview
SSP applies several IVM-specific optimizations to query plans...

## Predicate Pushdown
Filters are pushed into scan operators to enable O(1) delta filtering...

## Projection Pruning
Only fields actually used are fetched from tables...

## Usage
Optimizations are applied automatically by the converter...

## Performance Impact
- Predicate pushdown: 10-100x faster for selective filters
- Projection pruning: 20-50% memory savings
...
```

---

## Implementation Checklist

### Week 1
- [ ] Day 1: Extend Scan operator schema (Step 1.1)
- [ ] Day 2: Implement pushdown logic (Step 1.2)
- [ ] Day 2: Add pushdown tests (Step 1.3)
- [ ] Day 3: Update scan evaluation (Step 1.4)
- [ ] Day 3: Integration tests (Step 1.5)
- [ ] Day 4: Implement simplification (Step 2.1)
- [ ] Day 5: Add simplification tests (Step 2.2)

### Week 2
- [ ] Day 1: Implement field analysis (Step 3.1)
- [ ] Day 2: Update scan with projection (Step 3.2)
- [ ] Day 3: Projection tests (Step 3.3)
- [ ] Day 4: OPTIONAL: Join ordering (Step 4.1-4.2)
- [ ] Day 5: OPTIONAL: Join ordering tests (Step 4.3)

### Week 3
- [ ] Day 1-2: Comprehensive tests (Step 5.1)
- [ ] Day 3-4: Performance benchmarks (Step 5.2)
- [ ] Day 5: Documentation (Step 5.3)

---

## Success Metrics

### Correctness
- [ ] All existing tests still pass
- [ ] New optimization tests pass
- [ ] Integration tests demonstrate correct behavior

### Performance
- [ ] Selective filter queries: 10-100x faster
- [ ] Multi-join queries: 2-10x faster (if join ordering implemented)
- [ ] Memory usage: 20-50% reduction (with projection pruning)

### Code Quality
- [ ] No breaking changes to public API
- [ ] Clear documentation
- [ ] Maintainable code structure

---

## Rollback Plan

If any phase causes issues:

1. **Immediate rollback:**
```rust
pub fn convert_surql_to_dbsp(sql: &str) -> Result<Value> {
    let clean_sql = sql.trim().trim_end_matches(';');
    match parse_full_query(clean_sql) {
        Ok((_, plan)) => {
            // Add feature flag for easy disable
            let optimized = if cfg!(feature = "query-optimization") {
                optimize_plan(plan)
            } else {
                plan
            };
            Ok(optimized)
        }
        Err(e) => Err(anyhow!("SQL Parsing Error: {}", e)),
    }
}
```

2. **Add feature flag** to `Cargo.toml`:
```toml
[features]
default = ["query-optimization"]
query-optimization = []
```

3. **Disable if needed:**
```bash
cargo build --no-default-features
```

---

## Notes for Implementation

### Things I Know (From Your Code)
- You use `nom` for parsing ✓
- Query plans are JSON `Value` objects ✓
- You have `Operator` enum that deserializes from JSON ✓
- You have `wrap_conditions` function that separates joins/filters ✓

### Things You Need to Provide
- Location of `Operator` enum definition
- Location of scan/filter/join evaluation functions
- Your `Record` and `ZSet` type definitions
- Integration with your `Circuit` struct

### Key Principles
1. **Test incrementally** - don't implement everything at once
2. **Maintain backward compatibility** - use feature flags
3. **Benchmark before/after** - prove the optimizations work
4. **Document as you go** - explain why each optimization matters

---

## Questions to Answer Before Starting

1. **Where is your `Operator` enum defined?**
   - This is where you'll add `filter` field to `Scan` variant

2. **Where are operators evaluated?**
   - This is where you'll implement filter/projection logic

3. **How do you want to handle statistics for join ordering?**
   - Static config, dynamic from circuit, or skip for now?

4. **What's your testing strategy?**
   - Unit tests, integration tests, benchmarks?

---

## Summary

This plan gives you:
- ✅ **Concrete code** for converter changes (where I have full context)
- ✅ **Clear instructions** for evaluation changes (where I don't have context)
- ✅ **Phased approach** with independent deliverables
- ✅ **Testing strategy** at each step
- ✅ **Rollback plan** if issues arise

**Start with Phase 1 (Predicate Pushdown)** - it's the biggest win and easiest to implement.