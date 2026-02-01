# Query Plan Optimization Analysis: SSP Converter vs Apache DataFusion

## Executive Summary

Your current converter produces **list-based query plans** (sequences of operators), while Apache DataFusion uses **tree-based vectorized plans** with columnar execution. The key optimization opportunity is transitioning from row-at-a-time processing to columnar batch processing with SIMD.

**Impact Assessment:**
- **Complexity to Change:** Medium-High (requires architectural changes)
- **Performance Gain:** 5-10x for filter-heavy workloads, 2-3x for general queries
- **DBSP Module Impact:** Moderate - you'll need to extend operators, not rewrite core logic

---

## Part 1: Current Architecture Analysis

### Your Current Converter Output (List-Based)

```json
{
  "op": "limit",
  "input": {
    "op": "filter",
    "input": {
      "op": "scan",
      "table": "users"
    },
    "predicate": { "field": "age", "op": "gt", "value": 18 }
  },
  "count": 10
}
```

**Characteristics:**
- **Linear chain** of operators
- **Row-at-a-time** processing model implied
- **JSON-based** plan representation
- **Good for:** Simple IVM, debugging, small datasets
- **Bottleneck:** Each operator processes one row at a time

### Apache DataFusion Architecture (Tree-Based Columnar)

```rust
// DataFusion's LogicalPlan (simplified)
enum LogicalPlan {
    Scan { table: String, projection: Vec<usize> },
    Filter { predicate: Expr, input: Arc<LogicalPlan> },
    Limit { count: usize, input: Arc<LogicalPlan> },
    // ... 20+ operator types
}

// Physical execution plan (vectorized)
enum ExecutionPlan {
    CoalesceBatchesExec { input: Arc<dyn ExecutionPlan> },
    FilterExec { 
        predicate: Arc<dyn PhysicalExpr>,
        input: Arc<dyn ExecutionPlan>,
    },
    // Processes RecordBatch (Arrow columnar format)
}
```

**Characteristics:**
- **Tree structure** with `Arc<>` for shared nodes
- **Columnar batches** (Apache Arrow format)
- **SIMD-friendly** memory layout
- **Good for:** Large datasets, analytical queries, parallel execution
- **Bottleneck:** Higher memory overhead per batch

---

## Part 2: The Vectorization Opportunity

### What DataFusion Does Differently

#### 1. Columnar Layout

```rust
// Your current model (row-oriented)
struct Record {
    id: String,
    age: i64,
    name: String,
    active: bool,
}
// Processes: [Record1, Record2, Record3, ...]

// DataFusion model (column-oriented)
struct RecordBatch {
    columns: Vec<ArrayRef>,  // One array per column
    schema: SchemaRef,
}
// Processes batches of 1024-8192 rows at once
// Layout: [age: [18, 25, 30, ...], name: [...], ...]
```

**Why it's faster:**
- **CPU cache efficiency:** All ages are contiguous in memory
- **SIMD operations:** Process 4-16 values per instruction
- **Reduced branching:** Loop over primitive arrays, not structs

#### 2. SIMD Filter Example

```rust
// Your current approach (one-at-a-time)
fn filter_age_gt_18(records: &[Record]) -> Vec<Record> {
    records.iter()
        .filter(|r| r.age > 18)  // ← One comparison per iteration
        .cloned()
        .collect()
}

// DataFusion approach (vectorized)
fn filter_age_gt_18_simd(batch: &RecordBatch) -> RecordBatch {
    let age_col = batch.column(0).as_primitive::<Int64Type>();
    
    // SIMD: Compare 4-8 values at once
    let mask = arrow::compute::gt_scalar(age_col, 18)?;
    
    // Apply mask to all columns in one pass
    arrow::compute::filter_record_batch(batch, &mask)?
}
```

**Performance difference:**
- Your approach: ~1-2 cycles per record
- SIMD approach: ~0.2-0.5 cycles per record (4-8x faster)

#### 3. Expression Trees

```rust
// Your current predicate (JSON-based)
{
  "op": "and",
  "left": { "field": "age", "op": "gt", "value": 18 },
  "right": { "field": "active", "op": "eq", "value": true }
}

// DataFusion PhysicalExpr (compiled)
struct AndExpr {
    left: Arc<dyn PhysicalExpr>,
    right: Arc<dyn PhysicalExpr>,
}

impl PhysicalExpr for AndExpr {
    fn evaluate(&self, batch: &RecordBatch) -> Result<ColumnarValue> {
        let left_result = self.left.evaluate(batch)?;  // Returns boolean array
        let right_result = self.right.evaluate(batch)?;
        
        // SIMD bitwise AND on entire arrays
        arrow::compute::and(&left_result, &right_result)
    }
}
```

**Why this matters:**
- **Type safety:** Compile-time checking
- **Optimization:** Rust compiler can inline and vectorize
- **Avoid interpretation:** No runtime JSON parsing

---

## Part 3: Adapting to Your DBSP System

### Challenge: IVM Needs Fine-Grained Deltas

```rust
// DBSP operates on deltas
type Delta = HashMap<RecordId, (Record, Weight)>;

// Problem: Columnar batches don't map cleanly to sparse deltas
// Your system: Process 1 changed record
// DataFusion: Process 1024-8192 records in a batch
```

### Hybrid Solution: Vectorize Where It Matters

```
┌─────────────────────────────────────────────────┐
│         Your Query Plan (Enhanced)              │
├─────────────────────────────────────────────────┤
│                                                 │
│  ┌───────────────┐                             │
│  │  Scan (Base)  │  ← Still row-based for IVM  │
│  │  table: users │                              │
│  └───────┬───────┘                             │
│          │                                      │
│          ▼                                      │
│  ┌───────────────────┐                         │
│  │ VectorizedFilter  │  ← NEW: Batch processor │
│  │ batch_size: 1024  │                         │
│  │ predicate: ...    │                         │
│  └───────┬───────────┘                         │
│          │                                      │
│          ▼                                      │
│  ┌───────────────┐                             │
│  │  Limit (10)   │  ← Row-based for final step │
│  └───────────────┘                             │
│                                                 │
└─────────────────────────────────────────────────┘
```

**Strategy:**
- **Keep deltas row-based** (sparse updates work better this way)
- **Use vectorization for full scans** (initial view computation)
- **Batch filter predicates** when processing 100+ records

---

## Part 4: Implementation Roadmap

### Phase 1: Add Vectorized Filter Operator (Low Complexity)

**Changes needed in converter:**

```rust
// 1. Extend operator types
pub enum Operator {
    Scan { table: String },
    Filter { predicate: Predicate, input: Box<Operator> },
    
    // NEW: Vectorized variant
    VectorizedFilter {
        predicate: Predicate,
        input: Box<Operator>,
        batch_size: usize,  // e.g., 1024
    },
    
    Limit { count: usize, input: Box<Operator> },
    // ...
}

// 2. Add heuristic in converter
fn should_vectorize(predicate: &Predicate, estimated_rows: usize) -> bool {
    // Use vectorization if:
    // - Processing >100 rows
    // - Predicate is simple (eq, gt, lt - no complex logic)
    // - Not in a subquery (those are already filtered)
    
    estimated_rows > 100 && predicate.is_simple()
}

// 3. Modify parse_where_clause
fn parse_where_clause(...) -> Operator {
    let predicate = parse_predicate(...)?;
    
    if should_vectorize(&predicate, estimated_rows) {
        Operator::VectorizedFilter {
            predicate,
            input,
            batch_size: 1024,
        }
    } else {
        Operator::Filter { predicate, input }
    }
}
```

**Changes needed in DBSP eval:**

```rust
// In view.rs evaluation
match operator {
    Operator::Filter { predicate, input } => {
        // Current row-at-a-time logic
        let input_records = eval_operator(input, ...);
        input_records.into_iter()
            .filter(|r| eval_predicate(r, predicate))
            .collect()
    }
    
    Operator::VectorizedFilter { predicate, input, batch_size } => {
        // NEW: Batch processing logic
        let input_records = eval_operator(input, ...);
        
        // Convert to columnar batches
        let batches = records_to_batches(input_records, *batch_size);
        
        // Apply SIMD filter
        batches.into_iter()
            .flat_map(|batch| {
                let mask = eval_predicate_simd(&batch, predicate);
                filter_batch(batch, mask)
            })
            .collect()
    }
}
```

**DBSP Module Impact:** ⭐⭐⭐ (Moderate)
- Add ~200 lines for batch conversion utilities
- Add ~150 lines for SIMD predicate evaluation
- Existing delta propagation logic unchanged

---

### Phase 2: Arrow Integration (Medium Complexity)

If you want full DataFusion-style performance:

```toml
# Add dependencies
[dependencies]
arrow = "54.0"
arrow-array = "54.0"
arrow-schema = "54.0"
```

```rust
use arrow::array::{Int64Array, StringArray, BooleanArray};
use arrow::compute;

// 1. Extend SpookyValue to support Arrow arrays
pub enum SpookyValue {
    // Existing variants
    Null,
    Bool(bool),
    Number(f64),
    Str(SmolStr),
    Array(Vec<SpookyValue>),
    Object(FastMap<SmolStr, SpookyValue>),
    
    // NEW: Columnar batch
    Batch {
        schema: Arc<Schema>,
        columns: Vec<ArrayRef>,
        num_rows: usize,
    },
}

// 2. Add batch evaluation
fn eval_predicate_arrow(
    batch: &RecordBatch,
    predicate: &Predicate,
) -> Result<BooleanArray> {
    match predicate {
        Predicate::Gt { field, value } => {
            let col = batch.column_by_name(field)?;
            compute::gt_scalar(col, value)
        }
        Predicate::And { left, right } => {
            let left_mask = eval_predicate_arrow(batch, left)?;
            let right_mask = eval_predicate_arrow(batch, right)?;
            compute::and(&left_mask, &right_mask)
        }
        // ... other predicates
    }
}
```

**DBSP Module Impact:** ⭐⭐⭐⭐ (Moderate-High)
- Add Arrow dependency (~2MB binary size increase)
- Extend SpookyValue enum
- Add conversion functions (500-800 lines)
- Core IVM logic remains unchanged

---

### Phase 3: Tree-Based Plan (High Complexity)

If you want to match DataFusion's full architecture:

```rust
// 1. Change plan representation from JSON to tree
pub struct QueryPlan {
    root: Arc<Operator>,
    schema: Schema,
    statistics: Statistics,
}

pub enum Operator {
    Scan {
        table: SmolStr,
        projection: Option<Vec<usize>>,
        filter: Option<Arc<Predicate>>,  // Pushdown filter
    },
    Filter {
        predicate: Arc<Predicate>,
        input: Arc<Operator>,
    },
    Join {
        left: Arc<Operator>,
        right: Arc<Operator>,
        on: Vec<(usize, usize)>,  // Column indices
        join_type: JoinType,
    },
    // ...
}

// 2. Add query optimizer
pub struct Optimizer {
    rules: Vec<Box<dyn OptimizerRule>>,
}

trait OptimizerRule {
    fn optimize(&self, plan: Arc<Operator>) -> Result<Arc<Operator>>;
}

// Example: Predicate pushdown
struct PredicatePushdown;
impl OptimizerRule for PredicatePushdown {
    fn optimize(&self, plan: Arc<Operator>) -> Result<Arc<Operator>> {
        match plan.as_ref() {
            Operator::Filter { predicate, input } => {
                if let Operator::Scan { table, .. } = input.as_ref() {
                    // Push filter into scan
                    Ok(Arc::new(Operator::Scan {
                        table: table.clone(),
                        projection: None,
                        filter: Some(predicate.clone()),
                    }))
                } else {
                    Ok(plan)
                }
            }
            _ => Ok(plan),
        }
    }
}
```

**DBSP Module Impact:** ⭐⭐⭐⭐⭐ (High)
- Complete query plan refactor
- Add optimizer infrastructure (1000+ lines)
- Rewrite eval logic to handle tree recursion
- Benefits: Better optimization, easier to add features

---

## Part 5: Decision Framework

### Should You Vectorize?

```
┌─────────────────────────────────────────────────────────────┐
│                    Decision Tree                            │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  Q: What's your typical dataset size per view?             │
│     │                                                       │
│     ├─ <1K records  → Skip vectorization (overhead > gain) │
│     ├─ 1K-100K      → Phase 1 (selective vectorization)    │
│     └─ >100K        → Phase 2 (full Arrow integration)     │
│                                                             │
│  Q: What's your query pattern?                             │
│     │                                                       │
│     ├─ Mostly deltas       → Phase 1 only                  │
│     ├─ Mixed delta + scans → Phase 2                       │
│     └─ Analytical queries  → Phase 2 + Phase 3             │
│                                                             │
│  Q: Do you need sub-millisecond latency?                   │
│     │                                                       │
│     ├─ Yes → Stick with current (vectorization adds ~0.5ms)│
│     └─ No  → Vectorization is worth it                     │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### Performance vs Complexity Trade-off

```
Performance Gain vs Implementation Effort
                                        
  10x ┤                            ╭─────  Phase 3
      │                        ╭───╯
   8x ┤                    ╭───╯
      │                ╭───╯
   6x ┤            ╭───╯              Phase 2
      │        ╭───╯
   4x ┤    ╭───╯
      │╭───╯
   2x ┼╯                              Phase 1
      │
   1x ┼────────┬────────┬────────┬────────┬────────
      0      500     1000    1500    2000    2500
           Lines of Code to Implement
```

**Recommendation for SSP:**
- **Start with Phase 1** (selective vectorization)
- **Skip Phase 3** (tree-based plans) unless you need advanced optimization
- **Consider Phase 2** only if >50% of queries are full scans

---

## Part 6: DataFusion Techniques You Can Adopt NOW

### 1. Predicate Pushdown (No Vectorization Needed)

```rust
// Current converter output
{
  "op": "filter",
  "input": {
    "op": "scan",
    "table": "users"
  },
  "predicate": { "field": "id", "op": "eq", "value": "user:123" }
}

// Optimized: Push filter into scan
{
  "op": "scan",
  "table": "users",
  "filter": { "field": "id", "op": "eq", "value": "user:123" }
}
```

**Benefit:** Scan operator can use indexes or skip partitions
**Effort:** ~100 lines in converter

### 2. Projection Pruning

```rust
// Current: Fetch all columns
{
  "op": "scan",
  "table": "users"
}

// Optimized: Only fetch needed columns
{
  "op": "scan",
  "table": "users",
  "projection": ["id", "name"]  // Skip "bio", "created_at", etc.
}
```

**Benefit:** 50-80% less data to process
**Effort:** ~150 lines in converter + 50 lines in eval

### 3. Constant Folding

```rust
// Before
WHERE age > (10 + 8)

// After (in converter)
WHERE age > 18
```

**Benefit:** Avoid recomputing constants
**Effort:** ~50 lines

---

## Part 7: Recommended Implementation Plan

### Week 1: Quick Wins (No DBSP Changes)

1. **Predicate pushdown** (100 LOC)
   - Modify converter to detect pushable filters
   - Update eval to respect scan-level filters

2. **Projection pruning** (200 LOC)
   - Analyze SELECT fields in converter
   - Update scan to fetch only needed columns

3. **Constant folding** (50 LOC)
   - Add simplification pass in converter

**Expected gain:** 30-50% for queries with selective filters

### Week 2-3: Selective Vectorization (Phase 1)

1. **Add VectorizedFilter operator** (150 LOC in converter)
2. **Implement batch conversion** (200 LOC in view.rs)
3. **Add SIMD predicates** (300 LOC)
   - Start with simple: `eq`, `gt`, `lt`
   - Add `and`, `or` later

**Expected gain:** 2-3x for filter-heavy queries

### Month 2-3: Full Arrow Integration (Phase 2) - OPTIONAL

Only if you hit performance bottlenecks after Week 2-3.

---

## Part 8: Specific Answers to Your Questions

### Q: "Is this complicated to change?"

**A:** Depends on how deep you go:
- **Phase 1 (selective vectorization):** Moderate - ~1 week, 500-800 LOC
- **Phase 2 (full Arrow):** Significant - ~1 month, 2000-3000 LOC
- **Phase 3 (tree-based):** Very complex - ~2-3 months, 5000+ LOC refactor

### Q: "Do I have to change a lot in the DBSP IVM module?"

**A:** Not as much as you think:
- **Core delta propagation:** NO changes needed
- **ZSet algebra:** NO changes needed
- **View evaluation:** Add 300-500 LOC for batch path (Phase 1)
- **Arrow integration:** Add 800-1200 LOC (Phase 2)

**The key insight:** DBSP operates on deltas (sparse), vectorization helps with full scans (dense). You can keep both code paths.

### Q: "How does DataFusion do it differently?"

**A:** Three main differences:

1. **Columnar layout** - Groups same-typed values together
   - Your system: `[{id, age, name}, {id, age, name}, ...]`
   - DataFusion: `{ids: [...], ages: [...], names: [...]}`

2. **Expression compilation** - Rust types instead of JSON
   - Your system: `{"op": "gt", "field": "age", "value": 18}`
   - DataFusion: `Box<dyn PhysicalExpr>` compiled to machine code

3. **Batch processing** - Process 1000s of rows at once
   - Your system: `for record in records { eval(record) }`
   - DataFusion: `eval_batch(&records[0..1024])`  // SIMD-friendly

---

## Part 9: The Bottom Line

### For SSP Specifically:

**Do This Now (High ROI, Low Effort):**
- ✅ Predicate pushdown
- ✅ Projection pruning
- ✅ Constant folding

**Do This Later (Medium ROI, Medium Effort):**
- ⏸ Selective vectorization (Phase 1)
- ⏸ Only if queries consistently >1K rows

**Skip This (Low ROI for IVM):**
- ❌ Full Arrow integration (Phase 2)
- ❌ Tree-based plans (Phase 3)
- ❌ Unless you pivot to analytical workloads

### Why IVM Is Different from DataFusion:

| Aspect | DataFusion | Your SSP |
|--------|-----------|----------|
| **Workload** | Analytical queries | Real-time deltas |
| **Data size** | GBs-TBs per query | 1-1000 rows per update |
| **Latency target** | 100ms-10s | <10ms |
| **Vectorization win** | 5-10x | 1.5-2x (overhead eats gains) |

**Your system is already optimized for its use case** - sparse delta processing. DataFusion optimizes for dense batch processing.

---

## Conclusion

Vectorization like DataFusion is powerful but **not essential for your IVM workload**. Focus on:
1. Query-level optimizations (pushdown, pruning)
2. Keep row-based processing for deltas
3. Consider vectorization only for rare full-table scans

The DBSP module impact is moderate for Phase 1, but your current architecture is already well-suited to incremental computation. Don't chase DataFusion's architecture unless you're shifting to analytical queries.
