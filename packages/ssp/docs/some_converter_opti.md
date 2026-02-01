# Smart Query Plan Generation for DBSP/IVM - Converter Analysis

## Current Approach Analysis

Your converter builds query plans **bottom-up** (scan → filters → joins → projections → limit):

```rust
// Current flow in parse_full_query:
1. Start with scan: { "op": "scan", "table": table }
2. Apply WHERE: wrap_conditions(current_op, logic)
   - Separates joins from filters
   - Applies joins first (bottom-up)
   - Then applies filters
3. Add projections (if needed)
4. Add limit/order
```

**This is already pretty good!** But there are **smarter IVM-specific optimizations** you should add.

---

## The Smart Optimizations for IVM

### 1. **Predicate Pushdown** (CRITICAL for IVM)

**Current Problem:**
```json
{
  "op": "project",
  "projections": [...],
  "input": {
    "op": "filter",
    "predicate": { "type": "eq", "field": "id", "value": "thread:123" },
    "input": {
      "op": "scan",
      "table": "thread"
    }
  }
}
```

**IVM-Optimized:**
```json
{
  "op": "project",
  "projections": [...],
  "input": {
    "op": "scan",
    "table": "thread",
    "filter": { "type": "eq", "field": "id", "value": "thread:123" }
  }
}
```

**Why this matters for IVM:**
- When a delta arrives: `thread:123 updated`
- With pushdown: Check filter ONCE during scan (O(1))
- Without pushdown: Scan all threads, then filter (O(N))

**Implementation:**

```rust
fn optimize_predicate_pushdown(plan: Value) -> Value {
    match plan {
        Value::Object(mut map) => {
            let op = map.get("op").and_then(|v| v.as_str());
            
            // Pattern: filter(scan(table)) → scan(table, filter)
            if op == Some("filter") {
                if let Some(input) = map.get("input").and_then(|v| v.as_object()) {
                    if input.get("op").and_then(|v| v.as_str()) == Some("scan") {
                        // Check if predicate is "pushable"
                        if let Some(predicate) = map.get("predicate") {
                            if is_pushable_predicate(predicate) {
                                let mut scan = input.clone();
                                scan.insert("filter".to_string(), predicate.clone());
                                return json!(scan);
                            }
                        }
                    }
                }
            }
            
            // Recurse on children
            for (key, value) in &mut map {
                if key == "input" || key == "left" || key == "right" {
                    *value = optimize_predicate_pushdown(value.clone());
                }
            }
            
            Value::Object(map)
        }
        _ => plan,
    }
}

fn is_pushable_predicate(predicate: &Value) -> bool {
    // Pushable: eq, gt, lt, gte, lte, prefix (single-table predicates)
    // NOT pushable: joins, subqueries, complex logic with multiple tables
    
    match predicate.get("type").and_then(|v| v.as_str()) {
        Some("eq") | Some("gt") | Some("lt") | 
        Some("gte") | Some("lte") | Some("neq") | 
        Some("prefix") => true,
        
        Some("and") | Some("or") => {
            // Recursively check all predicates
            predicate.get("predicates")
                .and_then(|v| v.as_array())
                .map(|preds| preds.iter().all(is_pushable_predicate))
                .unwrap_or(false)
        }
        
        _ => false,
    }
}
```

---

### 2. **Join Ordering** (CRITICAL for Performance)

**Current Problem:**
Your `wrap_conditions` applies joins in the order they appear in WHERE clause.

```sql
-- Both of these produce DIFFERENT plans with current code:
WHERE thread.author = user.id AND post.thread_id = thread.id
-- vs --
WHERE post.thread_id = thread.id AND thread.author = user.id
```

**IVM-Optimized:**
Order joins by **selectivity** (smallest tables first):

```rust
fn optimize_join_order(joins: &[Value], base_table: &str) -> Vec<Value> {
    // Sort by estimated cardinality (smallest first)
    let mut ordered = joins.to_vec();
    ordered.sort_by_key(|join| {
        let right_table = join.get("right")
            .and_then(|v| v.get("table"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        
        estimate_table_size(right_table)
    });
    ordered
}

fn estimate_table_size(table: &str) -> usize {
    // You could maintain statistics, or use heuristics:
    match table {
        "user" => 10_000,      // Users table is usually large
        "session" => 50_000,   // Sessions very large
        "thread" => 5_000,     // Threads medium
        "comment" => 20_000,   // Comments large
        _ => 1_000,            // Unknown: assume small
    }
}
```

**Why this matters for IVM:**
- Join order affects intermediate result size
- Smaller intermediate results = less memory, faster processing
- Example: `users (10K) ⋈ posts (100K)` vs `posts (100K) ⋈ users (10K)`
  - Bad order: 100K × 10K = 1B comparisons
  - Good order: 10K × 100K with index = 10K lookups

---

### 3. **Subquery Flattening** (When Possible)

**Current:**
```json
{
  "op": "project",
  "projections": [
    { "type": "all" },
    {
      "type": "subquery",
      "alias": "author",
      "plan": {
        "op": "limit",
        "limit": 1,
        "input": {
          "op": "filter",
          "predicate": { "field": "id", "value": { "$param": "parent.author" } },
          "input": { "op": "scan", "table": "user" }
        }
      }
    }
  ]
}
```

**IVM-Optimized (when `LIMIT 1`):**
```json
{
  "op": "project",
  "projections": [{ "type": "all" }, { "type": "field", "name": "author.*" }],
  "input": {
    "op": "join",
    "type": "left_outer",  // Important: LEFT JOIN for optional
    "left": { "op": "scan", "table": "thread" },
    "right": { "op": "scan", "table": "user" },
    "on": { "left_field": "author", "right_field": "id" }
  }
}
```

**When to flatten:**
```rust
fn can_flatten_subquery(subquery: &Value) -> bool {
    // Conditions for safe flattening:
    // 1. LIMIT 1 (or no limit but guaranteed unique)
    // 2. Simple equality filter: id = $parent.field
    // 3. No aggregation
    
    let has_limit_1 = subquery.get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n == 1)
        .unwrap_or(false);
    
    let has_simple_filter = subquery.get("input")
        .and_then(|v| v.get("predicate"))
        .map(|p| is_simple_equality_filter(p))
        .unwrap_or(false);
    
    has_limit_1 && has_simple_filter
}
```

**Why this matters for IVM:**
- Joins are faster than nested subqueries
- Shared join state across multiple views
- Better optimization opportunities

---

### 4. **Projection Pruning** (Memory Optimization)

**Current Problem:**
You always select `*` unless explicitly specified otherwise.

**IVM-Optimized:**
Only fetch fields that are actually used:

```rust
fn analyze_used_fields(plan: &Value) -> HashSet<String> {
    let mut fields = HashSet::new();
    
    match plan.get("op").and_then(|v| v.as_str()) {
        Some("filter") => {
            // Extract fields from predicate
            extract_fields_from_predicate(
                plan.get("predicate").unwrap(), 
                &mut fields
            );
            // Recurse on input
            fields.extend(analyze_used_fields(
                plan.get("input").unwrap()
            ));
        }
        Some("project") => {
            // Only these fields are needed
            for proj in plan.get("projections")
                .and_then(|v| v.as_array())
                .unwrap_or(&vec![]) 
            {
                if let Some(field) = proj.get("name").and_then(|v| v.as_str()) {
                    fields.insert(field.to_string());
                }
            }
        }
        Some("join") => {
            // Need join key fields
            if let Some(on) = plan.get("on").and_then(|v| v.as_object()) {
                if let Some(left) = on.get("left_field").and_then(|v| v.as_str()) {
                    fields.insert(left.to_string());
                }
            }
        }
        _ => {}
    }
    
    fields
}

fn add_projection_pruning(plan: Value, needed_fields: &HashSet<String>) -> Value {
    // Modify scan operations to include projection
    match plan {
        Value::Object(mut map) => {
            if map.get("op").and_then(|v| v.as_str()) == Some("scan") {
                if !needed_fields.is_empty() && !needed_fields.contains("*") {
                    map.insert(
                        "projection".to_string(), 
                        json!(needed_fields.iter().collect::<Vec<_>>())
                    );
                }
            }
            Value::Object(map)
        }
        _ => plan,
    }
}
```

**Why this matters for IVM:**
- Less data to copy in ZSets
- Smaller memory footprint
- Faster comparisons (fewer fields to check)

---

### 5. **Constant Folding & Simplification**

**Current:**
```json
{
  "type": "and",
  "predicates": [
    { "type": "eq", "field": "active", "value": true },
    { "type": "eq", "field": "active", "value": true }  // Duplicate!
  ]
}
```

**IVM-Optimized:**
```rust
fn simplify_predicate(pred: Value) -> Value {
    match pred.get("type").and_then(|v| v.as_str()) {
        Some("and") | Some("or") => {
            let preds = pred.get("predicates")
                .and_then(|v| v.as_array())
                .unwrap();
            
            // Recursively simplify children
            let simplified: Vec<Value> = preds.iter()
                .map(|p| simplify_predicate(p.clone()))
                .collect();
            
            // Remove duplicates
            let mut unique = Vec::new();
            let mut seen = HashSet::new();
            for p in simplified {
                let key = serde_json::to_string(&p).unwrap();
                if seen.insert(key) {
                    unique.push(p);
                }
            }
            
            // Flatten nested AND/OR
            let flattened = flatten_logic(&unique, pred.get("type").unwrap().as_str().unwrap());
            
            if flattened.len() == 1 {
                flattened[0].clone()
            } else {
                json!({
                    "type": pred.get("type").unwrap(),
                    "predicates": flattened
                })
            }
        }
        _ => pred,
    }
}
```

---

## Implementation Strategy

### Add Optimization Pipeline

```rust
pub fn convert_surql_to_dbsp(sql: &str) -> Result<Value> {
    let clean_sql = sql.trim().trim_end_matches(';');
    
    // 1. Parse
    let (_, plan) = parse_full_query(clean_sql)
        .map_err(|e| anyhow!("SQL Parsing Error: {}", e))?;
    
    // 2. Optimize
    let optimized = optimize_plan(plan);
    
    Ok(optimized)
}

fn optimize_plan(plan: Value) -> Value {
    let mut optimized = plan;
    
    // Apply optimizations in order
    // (order matters - some enable others)
    
    // Phase 1: Algebraic simplification
    optimized = simplify_predicate(optimized);
    optimized = constant_folding(optimized);
    
    // Phase 2: Structural optimization
    optimized = optimize_predicate_pushdown(optimized);
    optimized = flatten_subqueries_where_possible(optimized);
    
    // Phase 3: Physical optimization  
    optimized = optimize_join_order(optimized);
    
    // Phase 4: Projection pruning
    let needed_fields = analyze_used_fields(&optimized);
    optimized = add_projection_pruning(optimized, &needed_fields);
    
    optimized
}
```

---

## Priority Recommendations

### MUST DO (High Impact, Low Effort):

1. **✅ Predicate Pushdown** (30 min implementation)
   - Single biggest IVM win
   - Reduces delta processing from O(N) to O(1) for filtered scans

2. **✅ Constant Folding** (20 min)
   - Free performance
   - Cleaner query plans

### SHOULD DO (Medium Impact, Medium Effort):

3. **⏸ Projection Pruning** (2 hours)
   - 20-50% memory savings
   - Faster ZSet operations

4. **⏸ Subquery Flattening** (3 hours)
   - Enables better optimization
   - Reduces nesting complexity

### COULD DO (High Impact, High Effort):

5. **⏸ Join Ordering** (4-6 hours)
   - Requires statistics/heuristics
   - Big win for multi-join queries
   - Can defer until you see performance issues

---

## Specific Code Changes for Your Converter

### Change 1: Add Predicate Pushdown to `wrap_conditions`

```rust
fn wrap_conditions(input_op: Value, predicate: Value) -> Value {
    let mut joins = Vec::new();
    let mut filters = Vec::new();
    
    // ... existing partitioning code ...
    
    let mut current_op = input_op;
    
    // NEW: Try to push filters into scan BEFORE applying joins
    if !filters.is_empty() && current_op.get("op") == Some(&json!("scan")) {
        let final_pred = if filters.len() == 1 {
            filters[0].clone()
        } else {
            json!({ "type": "and", "predicates": filters })
        };
        
        if is_pushable_predicate(&final_pred) {
            // Push into scan
            current_op.as_object_mut().unwrap()
                .insert("filter".to_string(), final_pred);
            filters.clear();  // Already applied
        }
    }
    
    // Apply Joins (existing code)
    for join_pred in joins {
        // ... existing join logic ...
    }
    
    // Apply remaining filters (if any weren't pushed)
    if !filters.is_empty() {
        // ... existing filter logic ...
    }
    
    current_op
}
```

### Change 2: Add Optimization Pass in `convert_surql_to_dbsp`

```rust
pub fn convert_surql_to_dbsp(sql: &str) -> Result<Value> {
    let clean_sql = sql.trim().trim_end_matches(';');
    match parse_full_query(clean_sql) {
        Ok((_, plan)) => {
            // NEW: Optimize before returning
            let optimized = optimize_predicate_pushdown(plan);
            Ok(optimized)
        }
        Err(e) => Err(anyhow!("SQL Parsing Error: {}", e)),
    }
}
```

---

## Performance Impact Estimates

### Before Optimization:
```sql
SELECT * FROM thread WHERE id = 'thread:123'
```
Delta arrives: Thread updated
- Scan all threads: O(N) - 10,000 threads
- Filter to find id=123: O(N) comparisons
- Time: ~100μs

### After Predicate Pushdown:
```sql
SELECT * FROM thread WHERE id = 'thread:123'
```
Delta arrives: Thread updated  
- Scan with filter: Check if delta.id == 'thread:123'
- If yes: O(1) lookup
- Time: ~1μs

**Result: 100x faster for filtered queries**

---

## The Bottom Line

Your current converter is **structurally sound**, but adding these optimizations will make your query plans **10-100x faster** for IVM:

1. **Start with predicate pushdown** - biggest bang for buck
2. **Add constant folding** - easy win
3. **Defer join ordering** - only if you see issues

The key insight: **IVM cares most about minimizing work on deltas**, so any optimization that reduces the "blast radius" of a single update is gold.
