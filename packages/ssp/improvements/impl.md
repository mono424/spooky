# Circuit Refactor Implementation Plan

## Goals
- Single record ingest (remove batch complexity)
- Cleaner struct separation
- Maintain performance as priority
- Better type safety

---

## Phase 1: Core Types

### 1.1 `Operation` enum
```rust
pub enum Operation {
    Create,
    Update,
    Delete,
}
```
- Replace string matching ("CREATE", "create", etc.)
- Zero-cost abstraction, single byte

### 1.2 `Record` struct
```rust
pub struct Record {
    pub table: SmolStr,
    pub id: SmolStr,
    pub data: SpookyValue,
    pub hash: String,
}
```
- Groups related fields
- Single parameter to `ingest()` instead of 5-tuple

### 1.3 `Delta` struct
```rust
pub struct Delta {
    pub table: SmolStr,
    pub key: SmolStr,
    pub weight: i64,
}
```
- Represents a single ZSet change
- Used for view propagation

---

## Phase 2: Table Refactor

### 2.1 Simplify `Table` methods
```rust
impl Table {
    fn apply(&mut self, op: Operation, key: SmolStr, data: SpookyValue, hash: String) -> Delta
}
```
- Single method handles create/update/delete
- Returns the delta it produced (for view propagation)
- Inlines weight calculation

### 2.2 Remove `apply_delta` from hot path
- `apply_delta` only used for ZSet bookkeeping
- Merge into `apply()` method

---

## Phase 3: Circuit Simplification

### 3.1 New `ingest` signature
```rust
pub fn ingest(&mut self, op: Operation, record: Record, is_optimistic: bool) -> Vec<ViewUpdate>
```
- Single record
- Clear intent via `Operation` enum
- Returns updates from affected views

### 3.2 `DependencyGraph` struct (optional extract)
```rust
struct DependencyGraph {
    map: FastMap<SmolStr, Vec<usize>>,
    valid: bool,
}

impl DependencyGraph {
    fn invalidate(&mut self)
    fn ensure_valid(&mut self, views: &[View])
    fn get_affected(&self, table: &SmolStr) -> &[usize]
}
```
- Encapsulates lazy rebuild logic
- Cleaner than inline checks

### 3.3 Simplified flow
```
ingest(op, record)
  │
  ├─► table.apply(op, ...) → Delta
  │
  ├─► dependency_graph.get_affected(table)
  │
  └─► for each affected view:
        view.process_single(delta, db) → Option<ViewUpdate>
```

---

## Phase 4: View Changes

### 4.1 New `process_single` method
```rust
impl View {
    pub fn process_single(&mut self, delta: &Delta, db: &Database, is_optimistic: bool) -> Option<ViewUpdate>
}
```
- Replaces `process_ingest` (batch) and `process` (step)
- Single delta in, optional update out

### 4.2 Remove batch-related code
- Delete `process_ingest` 
- Simplify internal delta handling

---

## Phase 5: Optional Enhancements

### 5.1 View index by ID
```rust
view_index: FastMap<SmolStr, usize>
```
- O(1) lookup for `set_record_version`, `unregister_view`
- Maintain on register/unregister

### 5.2 `IngestResult` struct
```rust
pub struct IngestResult {
    pub delta: Delta,
    pub view_updates: Vec<ViewUpdate>,
}
```
- Richer return type if needed for debugging/logging

---

## File Structure

```
circuit/
├── mod.rs          // re-exports
├── types.rs        // Operation, Record, Delta
├── table.rs        // Table, Database
├── dependency.rs   // DependencyGraph
├── circuit.rs      // Circuit (main orchestrator)
└── view.rs         // View (unchanged or minimal changes)
```

---

## Migration Steps

1. **Add new types** (Operation, Record, Delta) - non-breaking
2. **Add `Table::apply`** - parallel to existing methods
3. **Add `Circuit::ingest`** - new method alongside old
4. **Add `View::process_single`** - new method
5. **Wire up new flow** - `ingest` → `apply` → `process_single`
6. **Test new path** - verify correctness
7. **Remove old methods** - `ingest_batch`, `step`, old `process`
8. **Extract `DependencyGraph`** - optional cleanup

---

## Performance Notes

- **No allocations in hot path**: `Operation` is Copy, `Delta` is small
- **Single hash lookups**: `apply()` uses entry API
- **Inline hints**: on `apply`, `ingest`, `process_single`
- **Branch prediction**: `Operation` match is predictable
- **Keep parallel option**: can batch `ingest` calls externally if needed