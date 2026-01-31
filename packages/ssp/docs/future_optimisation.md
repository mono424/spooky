# SSP Circuit Optimization Analysis

## Executive Summary

This document analyzes five optimization strategies for the Spooky Stream Processor (SSP) DBSP circuit implementation. Each optimization is evaluated for implementation complexity, performance impact, risks, and recommended approach.

| Optimization | Effort | Impact | Priority | Recommendation |
|-------------|--------|--------|----------|----------------|
| Parallel Views | Low ✅ | High 🚀 | **DO NOW** | Already partially implemented with Rayon |
| Batch Ingestion | Low ✅ | Medium 📈 | **DO NOW** | Refine existing `ingest_batch` |
| View Caching | Medium | Medium 📈 | **Later** | Implement after profiling shows need |
| Circuit Sharding | High ⚠️ | High 🚀 | **Only if needed** | Multi-tenant or scale scenarios only |
| Message Queue | High ⚠️ | Medium 📈 | **Probably never** | DBSP fundamentally different from streaming |

---

## 1. Parallel Views

### Current State

Your implementation already has Rayon-based parallelism in `propagate_deltas`:

```rust
#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
{
    self.views
        .par_iter_mut()
        .enumerate()
        .filter_map(|(i, view)| {
            if impacted_view_indices.binary_search(&i).is_ok() {
                view.process_batch(batch_deltas, db_ref)
            } else {
                None
            }
        })
        .collect()
}
```

### What's Missing

#### 1.1 Parallel Table Mutations (Partially Done)

Your batch ingestion has parallel delta computation but sequential table ensuring:

```rust
// Current: Sequential table creation
for name in by_table.keys() {
    self.db.ensure_table(name.as_str());  // ← Sequential bottleneck
}

// Then parallel processing
let results: Vec<_> = self.db.tables.par_iter_mut()...
```

**Improvement**: Pre-allocate tables or use concurrent map:

```rust
// Option A: Pre-allocate all known tables at circuit creation
impl Circuit {
    pub fn with_tables(tables: &[&str]) -> Self {
        let mut circuit = Self::new();
        for t in tables {
            circuit.db.ensure_table(t);
        }
        circuit
    }
}

// Option B: Use DashMap for concurrent table access (adds dependency)
// pub tables: DashMap<String, Table>
```

#### 1.2 Subquery Parallelism

Nested subqueries currently execute sequentially within a view. For complex queries with independent subqueries, parallel execution could help:

```
SELECT * FROM threads WHERE author IN (
    SELECT id FROM users WHERE active = true  ← Could run in parallel
) AND category IN (
    SELECT id FROM categories WHERE public = true  ← with this
)
```

**Implementation Complexity**: High - requires query plan analysis to detect independent subqueries.

**Recommendation**: Skip for now. The view-level parallelism gives most benefit.

### Difficulties

| Challenge | Severity | Mitigation |
|-----------|----------|------------|
| WASM doesn't support threads | Medium | Already handled with `#[cfg]` gates |
| Rayon overhead for small batches | Low | Only parallelize when `views.len() > threshold` |
| Mutable borrow conflicts | Medium | Use `par_iter_mut` carefully, avoid shared state |
| Non-deterministic ordering | Low | Results collected into Vec, order doesn't matter |

### Recommended Actions (Priority: DO NOW)

1. **Add threshold gating** - Don't parallelize for < 4 views:
```rust
const PARALLEL_THRESHOLD: usize = 4;

if impacted_view_indices.len() >= PARALLEL_THRESHOLD {
    // Use par_iter_mut
} else {
    // Sequential iteration
}
```

2. **Benchmark native vs WASM** - Ensure WASM fallback isn't significantly slower

3. **Consider work-stealing tuning**:
```rust
// In lib.rs or main
rayon::ThreadPoolBuilder::new()
    .num_threads(num_cpus::get().min(8))  // Cap threads
    .build_global()
    .unwrap();
```

---

## 2. Batch Ingestion

### Current State

You have `ingest_batch` which groups by table and processes in one pass. This is already better than N×`ingest_single` calls.

### What's Missing

#### 2.1 Deferred Propagation

Currently each `ingest_batch` call triggers view propagation. For high-frequency updates (typing indicators, cursor positions), you want to batch across time windows:

```rust
pub struct Circuit {
    // ... existing fields
    pending_deltas: BatchDeltas,
    pending_tables: Vec<TableName>,
}

impl Circuit {
    /// Queue mutations without propagating
    pub fn ingest_deferred(&mut self, entries: Vec<BatchEntry>) {
        // Apply to storage, accumulate deltas
        // Don't call propagate_deltas
    }
    
    /// Flush all pending deltas to views
    pub fn flush(&mut self) -> Vec<ViewUpdate> {
        let deltas = std::mem::take(&mut self.pending_deltas);
        let tables = std::mem::take(&mut self.pending_tables);
        self.propagate_deltas(&deltas, &tables)
    }
}
```

**Use Case**: Frontend batches all mutations in a 16ms frame, calls `flush()` once per frame.

#### 2.2 Delta Coalescing

Multiple updates to same record within a batch should coalesce:

```rust
// Current: Each operation produces separate delta entry
// user:123 created (+1), then updated (content change), then deleted (-1)
// Results in 3 operations processed

// Optimized: Coalesce to net effect
// user:123: +1 -1 = 0 (no membership change, skip entirely)
```

**Implementation**:

```rust
fn coalesce_entries(entries: Vec<BatchEntry>) -> Vec<BatchEntry> {
    let mut by_key: FastMap<(TableName, SmolStr), BatchEntry> = FastMap::default();
    
    for entry in entries {
        let key = (entry.table.clone(), entry.id.clone());
        match by_key.entry(key) {
            Entry::Vacant(v) => { v.insert(entry); }
            Entry::Occupied(mut o) => {
                // Coalesce logic:
                // Create + Delete = remove entirely
                // Create + Update = Create with new data
                // Update + Delete = Delete
                // etc.
                let existing = o.get_mut();
                *existing = coalesce_ops(existing.clone(), entry);
            }
        }
    }
    
    by_key.into_values()
        .filter(|e| !e.is_noop())  // Remove Create+Delete pairs
        .collect()
}
```

### Difficulties

| Challenge | Severity | Mitigation |
|-----------|----------|------------|
| Ordering semantics | Medium | Document that batch operations are unordered |
| Memory growth in deferred mode | Medium | Add `pending_limit` with auto-flush |
| Coalescing correctness | High | Comprehensive tests for all operation combinations |
| Version tracking with coalescing | High | Keep highest version when coalescing |

### Recommended Actions (Priority: DO NOW)

1. **Implement deferred ingestion API** - Low effort, high value for frontend
2. **Add coalescing for same-record updates** - Prevents redundant work
3. **Add metrics**:
```rust
pub struct BatchStats {
    pub entries_received: usize,
    pub entries_after_coalesce: usize,
    pub tables_touched: usize,
    pub views_updated: usize,
}
```

---

## 3. View Caching

### Current State

Each view has:
- `cache: Vec<SpookyValue>` - Current materialized results
- `last_hash: SmolStr` - Hash for change detection
- Streaming mode bypasses full recomputation

### What's Missing

#### 3.1 Incremental Cache Updates

For append-only or simple filter views, you can patch the cache instead of recomputing:

```rust
enum CacheStrategy {
    /// Always recompute from scratch
    FullRecompute,
    
    /// Track insertions/deletions, patch cache
    Incremental {
        /// Records added since last full compute
        pending_inserts: Vec<SpookyValue>,
        /// Record IDs removed since last full compute
        pending_deletes: HashSet<SmolStr>,
    },
    
    /// For sorted views: maintain sort incrementally
    IncrementalSorted {
        sort_key: String,
        // Use BTreeMap for O(log n) insert maintaining order
    },
}
```

**When to use**:
- `FullRecompute`: Complex JOINs, aggregations, or small datasets
- `Incremental`: Simple filters on large datasets
- `IncrementalSorted`: Feeds, timelines, leaderboards

#### 3.2 Shared Subquery Cache

If multiple views have identical subqueries, compute once:

```
view_thread_detail: SELECT *, (SELECT * FROM users WHERE id = author_id) as author
view_thread_list:   SELECT *, (SELECT * FROM users WHERE id = author_id) as author
                              ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
                              Same subquery - compute once, share result
```

**Implementation**:

```rust
pub struct SubqueryCache {
    /// Key: normalized subquery SQL + params hash
    /// Value: (result, last_updated_tick)
    cache: FastMap<u64, (Vec<SpookyValue>, u64)>,
    current_tick: u64,
}

impl SubqueryCache {
    pub fn get_or_compute<F>(&mut self, key: u64, compute: F) -> &[SpookyValue]
    where F: FnOnce() -> Vec<SpookyValue>
    {
        self.cache.entry(key)
            .or_insert_with(|| (compute(), self.current_tick))
            .0.as_slice()
    }
    
    pub fn invalidate_for_table(&mut self, table: &str) {
        // Remove entries that reference this table
    }
}
```

### Difficulties

| Challenge | Severity | Mitigation |
|-----------|----------|------------|
| Cache invalidation correctness | **Critical** | Conservative invalidation, extensive tests |
| Memory overhead | Medium | LRU eviction, size limits |
| Incremental sort with complex keys | High | Fall back to full recompute |
| Subquery cache key generation | Medium | Hash normalized AST + params |

### Recommended Actions (Priority: LATER)

1. **Profile first** - Measure where time is actually spent
2. **Start with subquery deduplication** - Lower risk than incremental cache
3. **Add cache hit/miss metrics** before optimizing:
```rust
pub struct ViewMetrics {
    pub full_recomputes: u64,
    pub incremental_updates: u64,
    pub subquery_cache_hits: u64,
    pub subquery_cache_misses: u64,
}
```

---

## 4. Circuit Sharding

### When You Need This

- **Multi-tenant isolation**: Tenant A's slow query shouldn't block Tenant B
- **Horizontal scaling**: Single circuit can't handle update volume
- **Different SLAs**: Realtime views vs. analytics views with different latency requirements
- **Failure isolation**: One circuit crashing shouldn't affect others

### Architecture Options

#### 4.1 Tenant-Per-Circuit

```rust
pub struct ShardedCircuit {
    circuits: HashMap<TenantId, Circuit>,
    router: fn(&BatchEntry) -> TenantId,
}

impl ShardedCircuit {
    pub fn ingest(&mut self, entry: BatchEntry) {
        let tenant = (self.router)(&entry);
        self.circuits
            .entry(tenant)
            .or_insert_with(Circuit::new)
            .ingest_single(entry);
    }
}
```

**Pros**: Complete isolation, simple mental model
**Cons**: No cross-tenant queries, memory overhead per tenant

#### 4.2 Table-Per-Circuit (Sharding by Data)

```rust
pub struct TableShardedCircuit {
    /// High-frequency tables get dedicated circuits
    hot_tables: HashMap<TableName, Circuit>,
    /// Everything else
    default_circuit: Circuit,
}
```

**Pros**: Hot tables don't block cold tables
**Cons**: Cross-table JOINs become complex, need coordination

#### 4.3 Update-Frequency Sharding

```rust
pub struct FrequencyShardedCircuit {
    /// Realtime: cursor positions, typing indicators
    /// Stepped every mutation
    realtime: Circuit,
    
    /// Normal: messages, threads
    /// Stepped every 100ms or N mutations
    normal: Circuit,
    
    /// Analytics: aggregations, stats
    /// Stepped every 5 seconds
    analytics: Circuit,
}

impl FrequencyShardedCircuit {
    pub fn ingest(&mut self, entry: BatchEntry, frequency: UpdateFrequency) {
        match frequency {
            UpdateFrequency::Realtime => {
                self.realtime.ingest_single(entry);
            }
            UpdateFrequency::Normal => {
                self.normal.queue_deferred(entry);
            }
            UpdateFrequency::Analytics => {
                self.analytics.queue_deferred(entry);
            }
        }
    }
    
    pub fn tick(&mut self) {
        // Called from event loop
        self.normal.maybe_flush();      // Flush if 100ms passed
        self.analytics.maybe_flush();   // Flush if 5s passed
    }
}
```

### Difficulties

| Challenge | Severity | Mitigation |
|-----------|----------|------------|
| Cross-shard queries | **Critical** | Don't allow, or implement scatter-gather |
| Data duplication | High | Accept it, or implement shared storage layer |
| Consistency across shards | **Critical** | Accept eventual consistency or add coordination |
| Complexity explosion | High | Start with simplest sharding strategy |
| View registration routing | Medium | Views declare their shard affinity |

### Recommended Actions (Priority: ONLY IF NEEDED)

1. **Don't implement until you have proof of need**:
   - Single circuit processing > 10,000 updates/sec
   - Multi-tenant requirements with isolation SLAs
   - Latency requirements that conflict (realtime vs batch)

2. **If needed, start with frequency sharding** - Lowest complexity
3. **Measure single-circuit limits first** - You might be surprised how far it scales

---

## 5. Message Queue Integration

### Why You Probably Don't Need This

Your DBSP circuit is fundamentally different from message streaming:

| Aspect | Message Queue (Kafka/Redis) | DBSP Circuit |
|--------|----------------------------|--------------|
| Data model | Append-only log | Mutable state + deltas |
| Semantics | At-least-once delivery | Exactly-once computation |
| Ordering | Per-partition ordering | Transactional consistency |
| Query model | Consumer pulls messages | Views react to changes |
| Backpressure | Consumer lag | Synchronous processing |

### When It Might Make Sense

1. **Distributed deployment**: Multiple SSP instances need coordination
2. **Durability**: Mutations must survive crashes before processing
3. **External integration**: Other systems publish events you need to consume
4. **Replay capability**: Need to rebuild circuit state from event log

### Architecture If Needed

```
┌──────────────┐     ┌─────────────┐     ┌──────────────┐
│   Producers  │────►│   Message   │────►│  SSP Circuit │
│ (SurrealDB   │     │   Queue     │     │  (Consumer)  │
│  LIVE SELECT)│     │ (Redis/NATS)│     │              │
└──────────────┘     └─────────────┘     └──────────────┘
                            │
                            ▼
                     ┌─────────────┐
                     │  Persistent │
                     │    Log      │
                     │ (Recovery)  │
                     └─────────────┘
```

**Implementation Sketch**:

```rust
pub struct QueuedCircuit {
    circuit: Circuit,
    consumer: Box<dyn MessageConsumer>,
    pending: VecDeque<BatchEntry>,
    batch_size: usize,
    batch_timeout: Duration,
}

impl QueuedCircuit {
    pub async fn run(&mut self) {
        loop {
            // Collect messages up to batch_size or timeout
            let batch = self.collect_batch().await;
            
            // Process synchronously
            let updates = self.circuit.ingest_batch(batch);
            
            // Commit offset after successful processing
            self.consumer.commit().await;
            
            // Emit updates to subscribers
            self.emit_updates(updates).await;
        }
    }
}
```

### Difficulties

| Challenge | Severity | Mitigation |
|-----------|----------|------------|
| Exactly-once semantics | **Critical** | Idempotent processing + deduplication |
| Ordering guarantees | High | Single partition per table, or accept reordering |
| Latency overhead | Medium | In-process queue for local, network queue for distributed |
| Operational complexity | High | Need queue infrastructure, monitoring, alerting |
| WASM compatibility | **Critical** | No async runtime in WASM, need different architecture |

### Recommended Actions (Priority: PROBABLY NEVER)

1. **Don't add unless you have distributed requirements**
2. **If durability needed**: Write to SurrealDB first, use LIVE SELECT as "queue"
3. **If external integration needed**: Minimal adapter, not full queue system:
```rust
// Simple: Convert external events to BatchEntry, call ingest_batch
pub fn handle_external_event(event: ExternalEvent) -> BatchEntry {
    BatchEntry::new(event.table, event.op, event.id, event.data)
}
```

---

## Decision Framework

Use this flowchart to decide which optimizations to pursue:

```
START
  │
  ▼
┌─────────────────────────────────────┐
│ Is single-record latency too high?  │
└─────────────────────────────────────┘
  │ Yes                          │ No
  ▼                              ▼
┌──────────────┐          ┌──────────────────────────┐
│ Profile view │          │ Is throughput too low?   │
│ processing   │          └──────────────────────────┘
└──────────────┘            │ Yes              │ No
  │                         ▼                  ▼
  │                  ┌─────────────┐    ┌─────────────┐
  │                  │ Enable Rayon│    │ You're done │
  │                  │ parallelism │    │ for now     │
  │                  └─────────────┘    └─────────────┘
  ▼
┌───────────────────────────────────────┐
│ Is time spent in subquery execution?  │
└───────────────────────────────────────┘
  │ Yes                              │ No
  ▼                                  ▼
┌──────────────────┐          ┌─────────────────────────┐
│ Add subquery     │          │ Is time spent in cache  │
│ caching          │          │ recomputation?          │
└──────────────────┘          └─────────────────────────┘
                                │ Yes              │ No
                                ▼                  ▼
                         ┌──────────────┐   ┌──────────────┐
                         │ Implement    │   │ Check memory │
                         │ incremental  │   │ allocation   │
                         │ cache updates│   │ (SmolStr etc)│
                         └──────────────┘   └──────────────┘
```

---

## Implementation Roadmap

### Phase 1: Quick Wins (This Week)

- [ ] Add parallel threshold gating (don't parallelize < 4 views)
- [ ] Implement `ingest_deferred` + `flush` API
- [ ] Add basic metrics (updates/sec, views processed, batch sizes)

### Phase 2: Batch Optimization (Next 2 Weeks)

- [ ] Implement delta coalescing for same-record updates
- [ ] Add `BatchStats` return type with coalescing metrics
- [ ] Benchmark improvement with realistic workloads

### Phase 3: Profiling & Analysis (When Needed)

- [ ] Add flamegraph profiling to CI
- [ ] Identify actual bottlenecks with production-like data
- [ ] Decide on view caching strategy based on data

### Phase 4: Advanced (Only If Metrics Show Need)

- [ ] Subquery caching (if subquery time dominates)
- [ ] Incremental cache updates (if recomputation dominates)
- [ ] Circuit sharding (if isolation/scale required)

---

## Appendix: Benchmarking Template

Use this template to measure optimization impact:

```rust
#[cfg(test)]
mod benchmarks {
    use super::*;
    use std::time::Instant;
    
    fn setup_circuit(num_views: usize, records_per_table: usize) -> Circuit {
        let mut circuit = Circuit::new();
        // ... setup code
        circuit
    }
    
    #[test]
    fn bench_single_ingestion() {
        let mut circuit = setup_circuit(10, 1000);
        let entry = BatchEntry::update("users", "user:1", json!({"name": "test"}).into());
        
        let start = Instant::now();
        for _ in 0..1000 {
            circuit.ingest_single(entry.clone());
        }
        let elapsed = start.elapsed();
        
        println!("Single ingestion: {:?} per op", elapsed / 1000);
    }
    
    #[test]
    fn bench_batch_ingestion() {
        let mut circuit = setup_circuit(10, 1000);
        let entries: Vec<_> = (0..1000)
            .map(|i| BatchEntry::update("users", format!("user:{}", i), json!({"n": i}).into()))
            .collect();
        
        let start = Instant::now();
        circuit.ingest_batch(entries);
        let elapsed = start.elapsed();
        
        println!("Batch ingestion: {:?} total, {:?} per record", elapsed, elapsed / 1000);
    }
    
    #[test]
    fn bench_parallel_vs_sequential() {
        // Compare with/without Rayon feature
    }
}
```

---

## Summary

| Do Now | Do Later | Probably Never |
|--------|----------|----------------|
| Parallel threshold gating | View caching strategies | Message queue integration |
| Deferred ingestion API | Subquery deduplication | Full distributed architecture |
| Delta coalescing | Incremental cache updates | |
| Basic metrics | Circuit sharding | |

The key insight: **Your DBSP architecture is fundamentally sound**. The `dependency_list` already ensures O(affected views) not O(all views). Most "optimizations" add complexity without proportional benefit. Profile first, optimize second.
