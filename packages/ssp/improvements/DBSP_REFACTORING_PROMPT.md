# DBSP Engine Refactoring Prompt

## Objective

Refactor the DBSP engine module to combine the best architectural patterns from both versions while prioritizing **performance first**, then clean code, then flexibility.

## Context

You have two versions of a DBSP (Database Stream Processing) engine:
- **OLD version**: Better performance, clean abstractions (`Operation`, `BatchEntry`, `IngestBatch`), but no flexible versioning
- **NEW version**: Has `metadata.rs` with `VersionStrategy`, but lost performance optimizations and has code duplication

## Requirements (Priority Order)

### 1. PERFORMANCE (Highest Priority)

#### 1.1 Restore Operation Enum (circuit.rs)
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Operation {
    Create,
    Update,
    Delete,
}

impl Operation {
    #[inline]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "CREATE" => Some(Operation::Create),
            "UPDATE" => Some(Operation::Update),
            "DELETE" => Some(Operation::Delete),
            _ => None,
        }
    }

    #[inline]
    pub fn weight(&self) -> i64 {
        match self {
            Operation::Create | Operation::Update => 1,
            Operation::Delete => -1,
        }
    }

    #[inline]
    pub fn is_additive(&self) -> bool {
        matches!(self, Operation::Create | Operation::Update)
    }
}
```

#### 1.2 Restore BatchEntry with Optional Metadata
```rust
#[derive(Clone, Debug)]
pub struct BatchEntry {
    pub table: SmolStr,
    pub op: Operation,
    pub id: SmolStr,
    pub record: SpookyValue,
    pub hash: String,
    pub meta: Option<RecordMeta>,  // NEW: optional per-record metadata
}

impl BatchEntry {
    #[inline]
    pub fn new(
        table: impl Into<SmolStr>,
        op: Operation,
        id: impl Into<SmolStr>,
        record: SpookyValue,
        hash: String,
    ) -> Self {
        Self {
            table: table.into(),
            op,
            id: id.into(),
            record,
            hash,
            meta: None,
        }
    }

    #[inline]
    pub fn with_meta(mut self, meta: RecordMeta) -> Self {
        self.meta = Some(meta);
        self
    }

    #[inline]
    pub fn with_version(mut self, version: u64) -> Self {
        self.meta = Some(RecordMeta::new().with_version(version));
        self
    }

    pub fn from_tuple(tuple: (String, String, String, Value, String)) -> Option<Self> {
        let (table, op_str, id, record, hash) = tuple;
        let op = Operation::from_str(&op_str)?;
        Some(Self {
            table: SmolStr::from(table),
            op,
            id: SmolStr::from(id),
            record: SpookyValue::from(record),
            hash,
            meta: None,
        })
    }
}
```

#### 1.3 Restore IngestBatch Builder
```rust
pub struct IngestBatch {
    entries: Vec<BatchEntry>,
    default_strategy: Option<VersionStrategy>,
}

impl IngestBatch {
    #[inline]
    pub fn new() -> Self {
        Self { entries: Vec::new(), default_strategy: None }
    }

    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self { entries: Vec::with_capacity(capacity), default_strategy: None }
    }

    #[inline]
    pub fn with_strategy(mut self, strategy: VersionStrategy) -> Self {
        self.default_strategy = Some(strategy);
        self
    }

    #[inline]
    pub fn create(mut self, table: &str, id: &str, record: SpookyValue, hash: String) -> Self {
        self.entries.push(BatchEntry::new(table, Operation::Create, id, record, hash));
        self
    }

    #[inline]
    pub fn update(mut self, table: &str, id: &str, record: SpookyValue, hash: String) -> Self {
        self.entries.push(BatchEntry::new(table, Operation::Update, id, record, hash));
        self
    }

    #[inline]
    pub fn delete(mut self, table: &str, id: &str) -> Self {
        self.entries.push(BatchEntry::new(table, Operation::Delete, id, SpookyValue::Null, String::new()));
        self
    }

    // NEW: Version-aware builders
    #[inline]
    pub fn create_with_version(mut self, table: &str, id: &str, record: SpookyValue, hash: String, version: u64) -> Self {
        self.entries.push(BatchEntry::new(table, Operation::Create, id, record, hash).with_version(version));
        self
    }

    #[inline]
    pub fn update_with_version(mut self, table: &str, id: &str, record: SpookyValue, hash: String, version: u64) -> Self {
        self.entries.push(BatchEntry::new(table, Operation::Update, id, record, hash).with_version(version));
        self
    }

    #[inline]
    pub fn entry(mut self, entry: BatchEntry) -> Self {
        self.entries.push(entry);
        self
    }

    #[inline]
    pub fn build(self) -> (Vec<BatchEntry>, Option<VersionStrategy>) {
        (self.entries, self.default_strategy)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for IngestBatch {
    fn default() -> Self {
        Self::new()
    }
}
```

#### 1.4 Add #[inline] to ALL Hot Path Methods

Add `#[inline]` to these methods across all files:

**metadata.rs:**
- `ViewMetadataState::get_version()`
- `ViewMetadataState::set_version()`
- `ViewMetadataState::contains()`
- `ViewMetadataState::remove()`
- `ViewMetadataState::is_first_run()`
- `MetadataProcessor::compute_new_version()`
- `MetadataProcessor::compute_update_version()`

**view.rs:**
- `View::process()`
- `View::process_ingest()`
- `View::get_row_value()`
- `View::check_predicate()`

**circuit.rs:**
- `Table::update_row()`
- `Table::delete_row()`
- `Table::apply_delta()`
- `Circuit::ingest()`
- `Circuit::ingest_entries()`

#### 1.5 Avoid Cloning - Use References

```rust
// BEFORE (slow):
let processor = MetadataProcessor::new(self.metadata.strategy.clone());

// AFTER (fast):
impl<'a> MetadataProcessor<'a> {
    #[inline]
    pub fn new(strategy: &'a VersionStrategy) -> Self {
        Self { strategy }
    }
}

// Or even simpler - just inline the logic:
#[inline]
fn compute_version(&self, id: &str, is_new: bool, is_optimistic: bool, meta: Option<&RecordMeta>) -> u64 {
    let current = self.metadata.get_version(id);
    match self.metadata.strategy {
        VersionStrategy::Optimistic => {
            if is_new { 1 } else if is_optimistic { current + 1 } else { current }
        }
        VersionStrategy::Explicit => {
            meta.and_then(|m| m.version).unwrap_or(if is_new { 1 } else { current })
        }
        VersionStrategy::HashBased => current + 1,
        VersionStrategy::None => 0,
    }
}
```

#### 1.6 Use FastMap Everywhere

```rust
// BEFORE (slow):
pub type HashStore = HashMap<SmolStr, String>;

// AFTER (fast):
pub type HashStore = FastMap<SmolStr, String>;
```

#### 1.7 Pre-allocate Vectors

```rust
// When you know approximate size:
let mut additions = Vec::with_capacity(delta.len());
let mut removals = Vec::with_capacity(delta.len() / 4);  // Usually fewer removals

// Reserve in metadata:
if entries.len() > 10 {
    self.metadata.reserve(entries.len());
}
```

#### 1.8 Avoid Redundant Allocations

```rust
// BEFORE (wasteful):
let mut all_updates: Vec<ViewUpdate> = Vec::new();
// ... compute updates ...
all_updates.extend(updates);
all_updates

// AFTER (direct):
updates  // Just return directly
```

### 2. CLEAN CODE (Second Priority)

#### 2.1 Eliminate Code Duplication in circuit.rs

Create single internal method that handles all cases:

```rust
impl Circuit {
    // Public API methods (thin wrappers)
    pub fn ingest(&mut self, batch: IngestBatch, is_optimistic: bool) -> Vec<ViewUpdate> {
        let (entries, strategy) = batch.build();
        self.ingest_entries_internal(entries, strategy, is_optimistic)
    }

    pub fn ingest_entries(&mut self, entries: Vec<BatchEntry>, is_optimistic: bool) -> Vec<ViewUpdate> {
        self.ingest_entries_internal(entries, None, is_optimistic)
    }

    // Backward compatibility
    pub fn ingest_record(&mut self, table: &str, op: &str, id: &str, record: Value, hash: &str, is_optimistic: bool) -> Vec<ViewUpdate> {
        let op = match Operation::from_str(op) {
            Some(o) => o,
            None => return Vec::new(),
        };
        self.ingest_entries(vec![BatchEntry::new(table, op, id, SpookyValue::from(record), hash.to_string())], is_optimistic)
    }

    pub fn ingest_batch(&mut self, batch: Vec<(String, String, String, Value, String)>, is_optimistic: bool) -> Vec<ViewUpdate> {
        let entries: Vec<BatchEntry> = batch.into_iter().filter_map(BatchEntry::from_tuple).collect();
        self.ingest_entries(entries, is_optimistic)
    }

    // SINGLE internal implementation
    fn ingest_entries_internal(
        &mut self,
        entries: Vec<BatchEntry>,
        default_strategy: Option<VersionStrategy>,
        is_optimistic: bool,
    ) -> Vec<ViewUpdate> {
        if entries.is_empty() {
            return Vec::new();
        }

        // Build per-record metadata map from entries that have explicit meta
        let batch_meta = self.build_batch_meta(&entries, default_strategy);

        // Group by table for cache-friendly processing
        let mut by_table: FastMap<SmolStr, Vec<BatchEntry>> = FastMap::default();
        for entry in entries {
            by_table.entry(entry.table.clone()).or_default().push(entry);
        }

        let mut table_deltas: FastMap<String, ZSet> = FastMap::default();

        // Process each table's entries together
        for (table, table_entries) in by_table {
            let tb = self.db.ensure_table(table.as_str());
            let delta = table_deltas.entry(table.to_string()).or_default();

            for entry in table_entries {
                let weight = entry.op.weight();

                if entry.op.is_additive() {
                    tb.update_row(entry.id.clone(), entry.record, entry.hash);
                } else {
                    tb.delete_row(&entry.id);
                }

                *delta.entry(entry.id).or_insert(0) += weight;
            }
        }

        self.propagate_deltas(table_deltas, batch_meta, is_optimistic)
    }

    fn build_batch_meta(&self, entries: &[BatchEntry], default_strategy: Option<VersionStrategy>) -> Option<BatchMeta> {
        let has_any_meta = entries.iter().any(|e| e.meta.is_some()) || default_strategy.is_some();
        if !has_any_meta {
            return None;
        }

        let mut batch_meta = BatchMeta::new();
        if let Some(strategy) = default_strategy {
            batch_meta.default_strategy = strategy;
        }
        for entry in entries {
            if let Some(ref meta) = entry.meta {
                batch_meta.records.insert(entry.id.clone(), meta.clone());
            }
        }
        Some(batch_meta)
    }
}
```

#### 2.2 Restore ProcessContext in view.rs

```rust
/// Context for view processing - computed once, used throughout
struct ProcessContext<'a> {
    is_first_run: bool,
    is_streaming: bool,
    has_subquery_changes: bool,
    batch_meta: Option<&'a BatchMeta>,
}

impl<'a> ProcessContext<'a> {
    #[inline]
    fn new(view: &View, deltas: &FastMap<String, ZSet>, db: &Database, batch_meta: Option<&'a BatchMeta>) -> Self {
        let is_first_run = view.metadata.is_first_run();
        Self {
            is_first_run,
            is_streaming: matches!(view.format, ViewResultFormat::Streaming),
            has_subquery_changes: !is_first_run && view.has_changes_for_subqueries(deltas, db),
            batch_meta,
        }
    }

    #[inline]
    fn should_full_scan(&self) -> bool {
        self.is_first_run || self.has_subquery_changes
    }
}
```

#### 2.3 Use Structured Return Types

```rust
/// Result of change categorization
struct CategorizedChanges {
    delta: ZSet,
    additions: Vec<SmolStr>,  // Use SmolStr to avoid allocation
    removals: Vec<SmolStr>,
    updates: Vec<SmolStr>,
}

impl CategorizedChanges {
    #[inline]
    fn with_capacity(cap: usize) -> Self {
        Self {
            delta: FastMap::default(),
            additions: Vec::with_capacity(cap),
            removals: Vec::with_capacity(cap / 4),
            updates: Vec::with_capacity(cap / 2),
        }
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.additions.is_empty() && self.removals.is_empty() && self.updates.is_empty()
    }
}
```

### 3. FLEXIBILITY (Third Priority)

#### 3.1 Keep VersionStrategy Enum

```rust
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VersionStrategy {
    #[default]
    Optimistic,   // Auto-increment on update
    Explicit,     // Use version from RecordMeta
    HashBased,    // Derive from content hash
    None,         // No versioning (stateless)
}
```

#### 3.2 Keep ViewMetadataState

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ViewMetadataState {
    #[serde(default)]
    pub versions: VersionMap,
    #[serde(default, skip_serializing_if = "FastMap::is_empty")]
    pub hashes: HashStore,  // Use FastMap!
    #[serde(default)]
    pub strategy: VersionStrategy,
    #[serde(default)]
    pub last_result_hash: String,
}

impl ViewMetadataState {
    #[inline]
    pub fn new(strategy: VersionStrategy) -> Self {
        Self { strategy, ..Default::default() }
    }

    #[inline]
    pub fn get_version(&self, id: &str) -> u64 {
        self.versions.get(id).copied().unwrap_or(0)
    }

    #[inline]
    pub fn set_version(&mut self, id: impl Into<SmolStr>, version: u64) {
        self.versions.insert(id.into(), version);
    }

    #[inline]
    pub fn remove(&mut self, id: &str) {
        self.versions.remove(id);
        self.hashes.remove(id);
    }

    #[inline]
    pub fn contains(&self, id: &str) -> bool {
        self.versions.contains_key(id)
    }

    #[inline]
    pub fn is_first_run(&self) -> bool {
        self.last_result_hash.is_empty() && self.versions.is_empty()
    }

    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.versions.reserve(additional);
    }
}
```

#### 3.3 Simplified Version Computation (Inline, No Processor Struct)

```rust
impl View {
    /// Compute and store version for a record
    #[inline]
    fn compute_and_store_version(
        &mut self,
        id: &str,
        is_new: bool,
        is_optimistic: bool,
        ctx: &ProcessContext,
    ) -> u64 {
        let current = self.metadata.get_version(id);
        
        // Check for explicit version in batch metadata
        if let Some(batch_meta) = ctx.batch_meta {
            if let Some(record_meta) = batch_meta.get(id) {
                if let Some(explicit_version) = record_meta.version {
                    self.metadata.set_version(id, explicit_version);
                    return explicit_version;
                }
            }
            // Check default strategy override
            if batch_meta.default_strategy == VersionStrategy::Explicit {
                // Explicit mode but no version provided - keep current or 1
                let v = if is_new { 1 } else { current };
                self.metadata.set_version(id, v);
                return v;
            }
        }

        // Use view's strategy
        let version = match self.metadata.strategy {
            VersionStrategy::Optimistic => {
                if is_new { 
                    1 
                } else if is_optimistic { 
                    current.saturating_add(1) 
                } else { 
                    current 
                }
            }
            VersionStrategy::Explicit => {
                // No explicit version provided, keep current or default
                if is_new { 1 } else { current }
            }
            VersionStrategy::HashBased => {
                // For hash-based, caller should handle hash comparison
                // Here we just increment if changed
                if is_new { 1 } else { current.saturating_add(1) }
            }
            VersionStrategy::None => 0,
        };

        if version != current || is_new {
            self.metadata.set_version(id, version);
        }
        version
    }
}
```

### 4. ADDITIONAL OPTIMIZATIONS

#### 4.1 Use SmolStr for Short-Lived Strings

```rust
// In CategorizedChanges, use SmolStr instead of String
// SmolStr is stack-allocated for strings <= 23 bytes (most IDs)
additions: Vec<SmolStr>,
removals: Vec<SmolStr>,
updates: Vec<SmolStr>,
```

#### 4.2 Avoid HashSet Creation in Hot Paths

```rust
// BEFORE (allocates HashSet every call):
let updated_ids_set: HashSet<&str> = updated_record_ids.iter().map(|s| s.as_str()).collect();

// AFTER (use sorted vec + binary_search for small sets):
fn contains_sorted(sorted: &[SmolStr], id: &str) -> bool {
    sorted.binary_search_by(|s| s.as_str().cmp(id)).is_ok()
}

// Or for larger sets, reuse a scratch HashSet:
struct ViewScratch {
    updated_set: HashSet<SmolStr>,
}
```

#### 4.3 Use sort_unstable Instead of sort

```rust
// BEFORE:
items.sort_by(|a, b| a.0.cmp(&b.0));

// AFTER (faster, no stability needed for IDs):
items.sort_unstable_by(|a, b| a.0.cmp(&b.0));
```

#### 4.4 Batch Metadata Updates

```rust
impl ViewMetadataState {
    /// Batch update versions (avoids repeated hash lookups)
    #[inline]
    pub fn set_versions_batch(&mut self, items: impl IntoIterator<Item = (SmolStr, u64)>) {
        for (id, version) in items {
            self.versions.insert(id, version);
        }
    }

    /// Batch remove (avoids repeated hash lookups)  
    #[inline]
    pub fn remove_batch(&mut self, ids: impl IntoIterator<Item = impl AsRef<str>>) {
        for id in ids {
            self.versions.remove(id.as_ref());
            self.hashes.remove(id.as_ref());
        }
    }
}
```

#### 4.5 Lazy Subquery Table Extraction

```rust
// BEFORE (computed even when not needed):
let subquery_tables: HashSet<String> = self.extract_subquery_tables(&self.plan.root).into_iter().collect();

// AFTER (lazy, cached):
impl View {
    // Cache subquery tables (computed once per view)
    #[serde(skip)]
    subquery_tables_cache: Option<HashSet<SmolStr>>,

    fn get_subquery_tables(&mut self) -> &HashSet<SmolStr> {
        if self.subquery_tables_cache.is_none() {
            self.subquery_tables_cache = Some(
                self.extract_subquery_tables(&self.plan.root)
                    .into_iter()
                    .map(SmolStr::from)
                    .collect()
            );
        }
        self.subquery_tables_cache.as_ref().unwrap()
    }
}
```

#### 4.6 Parallel View Threshold Tuning

```rust
// Current threshold might be too high for some workloads
const PARALLEL_VIEW_THRESHOLD: usize = 10;

// Consider dynamic threshold based on delta size:
fn should_parallelize(view_count: usize, total_delta_size: usize) -> bool {
    view_count >= 4 && total_delta_size > 100
}
```

### 5. API COMPATIBILITY

Maintain backward compatibility with these signatures:

```rust
// Old API (must work)
circuit.ingest_record(table, op, id, record, hash, is_optimistic)
circuit.ingest_batch(vec![(table, op, id, record, hash)], is_optimistic)
circuit.register_view(plan, params, format)  // 3 params!

// New API (additions)
circuit.ingest(IngestBatch::new().create(...).update(...), is_optimistic)
circuit.ingest(IngestBatch::new().with_strategy(Explicit).update_with_version(..., 13), false)
circuit.register_view_with_strategy(plan, params, format, strategy)  // 4 params, new name!
```

### 6. FILE STRUCTURE

```
engine/
├── mod.rs              # Re-exports
├── circuit.rs          # Circuit, Database, Table, Operation, BatchEntry, IngestBatch
├── view.rs             # View, QueryPlan, ProcessContext, CategorizedChanges
├── metadata.rs         # VersionStrategy, RecordMeta, BatchMeta, ViewMetadataState
├── update.rs           # ViewUpdate, RawViewResult, ViewDelta, build_update()
├── types/
│   ├── mod.rs
│   ├── zset.rs         # FastMap, ZSet, Weight, RowKey
│   ├── path.rs         # Path
│   └── spooky_value.rs # SpookyValue
├── operators/
│   ├── mod.rs
│   ├── operator.rs     # Operator enum
│   ├── predicate.rs    # Predicate enum
│   └── projection.rs   # Projection enum
└── eval/
    ├── mod.rs
    └── filter.rs       # SIMD filter, comparison functions
```

### 7. TESTING REQUIREMENTS

After refactoring, ensure these tests pass:

```rust
#[test]
fn test_backward_compat_ingest_record() {
    let mut circuit = Circuit::new();
    circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));
    let updates = circuit.ingest_record("users", "CREATE", "user:1", json!({}), "hash", true);
    assert!(!updates.is_empty());
}

#[test]
fn test_builder_api() {
    let mut circuit = Circuit::new();
    circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));
    let updates = circuit.ingest(
        IngestBatch::new()
            .create("users", "user:1", data, hash)
            .update("users", "user:2", data, hash),
        true
    );
    assert!(!updates.is_empty());
}

#[test]
fn test_explicit_version() {
    let mut circuit = Circuit::new();
    circuit.register_view_with_strategy(plan, None, Some(ViewResultFormat::Streaming), VersionStrategy::Explicit);
    
    let updates = circuit.ingest(
        IngestBatch::new()
            .with_strategy(VersionStrategy::Explicit)
            .update_with_version("users", "user:1", data, hash, 42),
        false  // is_optimistic ignored for Explicit
    );
    
    if let ViewUpdate::Streaming(s) = &updates[0] {
        assert_eq!(s.records[0].version, 42);  // Must be explicit version!
    }
}

#[test]
fn test_optimistic_increment() {
    let mut circuit = Circuit::new();
    circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));
    
    circuit.ingest_record("users", "CREATE", "user:1", json!({}), "h1", true);
    let updates = circuit.ingest_record("users", "UPDATE", "user:1", json!({}), "h2", true);
    
    if let ViewUpdate::Streaming(s) = &updates[0] {
        assert_eq!(s.records[0].version, 2);  // Auto-incremented
    }
}

#[test]
fn test_non_optimistic_keeps_version() {
    let mut circuit = Circuit::new();
    circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));
    
    circuit.ingest_record("users", "CREATE", "user:1", json!({}), "h1", true);
    let updates = circuit.ingest_record("users", "UPDATE", "user:1", json!({}), "h2", false);
    
    if let ViewUpdate::Streaming(s) = &updates[0] {
        assert_eq!(s.records[0].version, 1);  // NOT incremented
    }
}
```

## Summary Checklist

### Must Do (Performance)
- [ ] Restore `Operation` enum with `#[inline]` methods
- [ ] Restore `BatchEntry` struct with optional `meta` field
- [ ] Restore `IngestBatch` builder with version-aware methods
- [ ] Add `#[inline]` to all hot path methods
- [ ] Change `HashStore` to use `FastMap`
- [ ] Remove redundant allocations (all_updates vector)
- [ ] Use references instead of cloning in MetadataProcessor
- [ ] Pre-allocate vectors with capacity

### Should Do (Clean Code)
- [ ] Single `ingest_entries_internal()` method (no duplication)
- [ ] Restore `ProcessContext` struct
- [ ] Use `CategorizedChanges` struct for return values
- [ ] Remove duplicate comments

### Nice to Have (Flexibility)
- [ ] Keep `VersionStrategy` enum
- [ ] Keep `ViewMetadataState`
- [ ] Keep `BatchMeta` for explicit versions
- [ ] Add `register_view_with_strategy()` (don't break old API)

### Additional Optimizations
- [ ] Use `SmolStr` in hot paths
- [ ] Cache subquery tables
- [ ] Use `sort_unstable`
- [ ] Batch metadata updates
- [ ] Consider binary_search for small sets instead of HashSet
