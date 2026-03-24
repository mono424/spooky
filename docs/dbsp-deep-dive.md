# From Theory to Engine: A Deep Dive into Database Stream Processors (DBSP)

*How the mathematics of Z-sets, incremental view maintenance, and circuit computation power real-time reactive systems — with lessons from building one in Rust.*

> This document is designed to be read linearly, like a lecture or a podcast episode. By the end, you will understand the full theory of DBSP, see exactly how it maps to a working Rust implementation (the Sp00ky Stream Processor), and have concrete recommendations for building or rebuilding a DBSP-based system. If you are converting this to audio, it reads well from start to finish.

---

## Part 1: The Problem — Why Your Database Is Always Behind

Imagine you open a social app. You see a list of threads. Someone posts a new message. Your list should update instantly. In a traditional system, this means re-running the entire query — scanning every thread, checking every filter, sorting, limiting, and sending the full result set back to your screen. For a table with ten rows, that is fine. For a table with a hundred thousand rows and fifty active views watching it, that is a disaster.

This is the fundamental problem: queries are computed once, but data changes continuously. The moment you run `SELECT * FROM thread WHERE status = 'active' ORDER BY created_at DESC LIMIT 10`, the result is already potentially stale. Someone might have posted a new thread, or deleted one, or changed a thread's status from "active" to "archived." Your materialized view — that snapshot of query results sitting in memory or on screen — is a photograph of a river. The river has already moved on.

The brute-force solution is to re-run the query on every change. But that means every single insert, update, or delete on the `thread` table triggers a full table scan, a full filter pass, a full sort, and a full result set transmission. If you have fifty views watching that table, you are doing fifty full scans per mutation. The computational cost grows linearly with both the number of records and the number of views. This does not scale.

The elegant solution is called **Incremental View Maintenance**, or IVM. The idea is beautifully simple: instead of re-running the whole query from scratch, you compute *only what changed*. If one record was inserted, you figure out whether that record affects the view, and if so, you emit a tiny delta — "this one record was added" — rather than re-transmitting the entire view.

The challenge is making this work for arbitrary queries. Filtering is easy: does the new record pass the filter? Yes or no. But what about joins? What about aggregations like `COUNT(*)`? What about `LIMIT 10` where inserting one record might push another record out of the top ten? These cases are subtle, and getting them wrong means your view silently diverges from the truth.

This is where DBSP comes in. **Database Stream Processing**, or DBSP, is the most rigorous mathematical framework for incremental view maintenance ever published. It was developed by Mihai Budiu, Frank McSherry, and their team (first at VMware Research, now at Feldera). Their key contribution, published at VLDB 2023, is a proof that *any* relational query — including joins, aggregations, nested queries, and recursive queries — can be incrementalized using a small set of mathematical primitives. Not "most queries." Not "simple queries." Any query.

### The Sp00ky Context

This is not just academic for us. Sp00ky is a reactive, local-first framework for SurrealDB. Its beating heart is the **SSP** — the Sp00ky Stream Processor — a Rust crate that implements a DBSP-inspired incremental view maintenance engine. The SSP runs in two environments: as a WebAssembly module inside the browser (so your UI updates in real-time without round trips to a server) and as a native Rust sidecar service on the backend (where it processes record changes from SurrealDB and broadcasts view updates to connected clients).

The SSP currently handles the basics: ingesting record changes, maintaining materialized views defined by SurrealQL queries, computing deltas, and emitting updates. But it is not working correctly in its current state. The incremental evaluation is incomplete — it falls back to full re-computation for complex queries — and the codebase has accumulated patches that need a clean rebuild. Before we rebuild, we need to deeply understand the theory. That is what this document is for.

*So to recap: the problem is stale views. The solution is incremental view maintenance. The framework is DBSP. And the implementation we are building is the Sp00ky Stream Processor. Now let us dig into the mathematics that makes it all work.*

---

## Part 2: Z-Sets — The Mathematical Foundation

If DBSP has one core idea that everything else builds on, it is the **Z-set**. Understanding Z-sets is like understanding the number line before doing algebra — everything else follows from this foundation.

### What Is a Z-Set?

You are probably familiar with sets and multisets. A set is a collection of unique elements: `{alice, bob, charlie}`. A multiset (also called a bag) allows duplicates: `{alice, alice, bob}` — alice appears twice. You can think of a multiset as a function that maps each element to a non-negative integer count: `{alice: 2, bob: 1}`.

A Z-set takes this one step further. It is a function from a domain D to the integers Z (hence the name), where elements can have *negative* counts. So `{alice: 1, bob: -1, charlie: 2}` is a perfectly valid Z-set. Alice is present once. Charlie is present twice. And Bob? Bob has a weight of -1.

What does a negative weight mean? This is the key insight. A negative weight represents a *deletion*. If the current state of a table is `{alice: 1, bob: 1}` and Bob gets deleted, the *delta* (the change) is `{bob: -1}`. When you add the delta to the state, you get `{alice: 1, bob: 0}` — and since a weight of zero means "not present," Bob is gone.

Think of it like a bank ledger. Deposits are positive numbers. Withdrawals are negative numbers. The balance is the running sum. A Z-set is exactly this, but for database records instead of dollars. Every insertion is a deposit. Every deletion is a withdrawal. The current state of the table is the running balance.

Here is a concrete walk-through. Let us track a `user` table through several mutations:

```
Step 0 (empty table):
  State:  {}

Step 1 (create alice):
  Delta:  {"user:alice": +1}
  State:  {"user:alice": 1}

Step 2 (create bob):
  Delta:  {"user:bob": +1}
  State:  {"user:alice": 1, "user:bob": 1}

Step 3 (delete alice):
  Delta:  {"user:alice": -1}
  State:  {"user:bob": 1}

Step 4 (create alice again):
  Delta:  {"user:alice": +1}
  State:  {"user:alice": 1, "user:bob": 1}
```

Notice how the state at each step is simply the sum of all previous deltas. And notice how each delta only contains the records that actually changed. This is exactly what we need for incremental processing — the delta is always small, even if the state is large.

### The Abelian Group Structure

Here is where the math gets beautiful. Z-sets form an **abelian group** under pointwise addition. If you have not thought about abstract algebra since university, do not worry — the intuition is straightforward. An abelian group just means four things are true:

First, **addition is associative and commutative**. If you add delta A to delta B, or delta B to delta A, you get the same result. The order does not matter. This means you can batch up changes and apply them in any order.

Second, there is an **identity element**: the empty Z-set `{}`. Adding the empty Z-set to anything changes nothing.

Third, every Z-set has an **inverse**. The inverse of `{alice: 1, bob: -1}` is `{alice: -1, bob: 1}` — you just negate all the weights. Adding a Z-set to its inverse gives you the empty Z-set.

Fourth, you can **subtract** Z-sets to get deltas. If you have the old state and the new state, the delta is `new - old` (pointwise subtraction). This is exactly the `diff` operation.

Why does this matter for engineering? Because the group structure guarantees that delta operations compose correctly. You can split a batch of changes into sub-batches, process them independently, merge the results, and the final state will be identical to processing them sequentially. This is what enables parallelism.

### Z-Sets in the Sp00ky Codebase

If you look at the SSP source code in `packages/ssp/src/engine/types/zset.rs`, you will see the Z-set definition is almost comically simple:

```rust
pub type Weight = i64;
pub type RowKey = SmolStr;
pub type FastMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FxHasher>>;
pub type ZSet = FastMap<RowKey, Weight>;
```

A Z-set is literally a `HashMap` from string keys to integer weights. The keys are strings of the form `"table:id"` — for example, `"user:alice"` or `"thread:01HXYZ"`. The weights are 64-bit signed integers. We use `FxHasher` instead of the default hasher for performance, and `SmolStr` instead of `String` for small-string optimization (strings of 23 bytes or fewer are stored inline without heap allocation).

The `ZSetOps` trait implements the core DBSP operations on this type:

```rust
pub trait ZSetOps {
    /// Add delta to ZSet: result[k] = self[k] + delta[k]
    fn add_delta(&mut self, delta: &ZSet);

    /// Compute difference: result[k] = other[k] - self[k]
    fn diff(&self, other: &ZSet) -> ZSet;

    /// Check if record is present (weight > 0)
    fn is_present(&self, key: &str) -> bool;

    /// Get records that transitioned to/from presence
    fn membership_changes(&self, other: &ZSet) -> Vec<(SmolStr, WeightTransition)>;
}
```

The `add_delta` method is the group operation — pointwise addition. The `diff` method computes the subtraction `other - self`, which gives you the delta between two states. And `membership_changes` detects when a record crosses the threshold from absent (weight <= 0) to present (weight > 0) or vice versa, using the `WeightTransition` enum:

```rust
pub enum WeightTransition {
    Inserted,              // old_weight <= 0, new_weight > 0
    MultiplicityIncreased, // old_weight > 0, new_weight > old_weight
    MultiplicityDecreased, // new_weight > 0, new_weight < old_weight
    Deleted,               // old_weight > 0, new_weight <= 0
    Unchanged,
}
```

### The Membership Simplification

The codebase also defines a second trait, `ZSetMembershipOps`, which normalizes weights to either 0 (absent) or 1 (present). This is a practical simplification. In many real-world scenarios — like tracking which records are in a view — you only care about *whether* a record is in the set, not *how many times* it appears. The membership model treats `add_member` as "set weight to 1" and `remove_member` as "remove the key entirely."

```rust
pub trait ZSetMembershipOps {
    fn is_member(&self, key: &str) -> bool;
    fn add_member(&mut self, key: SmolStr);
    fn remove_member(&mut self, key: &str) -> bool;
    fn apply_membership_delta(&mut self, delta: &ZSet);
    fn membership_diff(&self, target: &ZSet) -> (Vec<SmolStr>, Vec<SmolStr>);
    fn normalize_to_membership(&mut self);
    fn member_count(&self) -> usize;
}
```

The trade-off here is important. The membership model is correct and efficient for tracking "which records does this view contain." But if you ever need aggregation queries like `SELECT COUNT(*) FROM user WHERE status = 'active'` or `SELECT status, COUNT(*) FROM user GROUP BY status`, you need the full multiplicity model — because the count depends on how many times each record appears, not just whether it appears at all. The rebuild should keep the full Z-set algebra as the core and layer the membership simplification on top as an optimization for views that only need presence tracking.

### The Ring Structure

For the mathematically curious: Z-sets actually form a **ring**, not just a group. A ring adds a multiplication operation on top of addition. For Z-sets, multiplication is defined as: for each pair of elements `(a, b)` where `a` comes from Z-set A and `b` comes from Z-set B, the output weight is the product of their weights.

This is exactly how joins work in DBSP. When you join table A with table B, the output weight of a matched pair is `weight_A * weight_B`. You can see this directly in the SSP's join implementation in `packages/ssp/src/engine/view.rs`:

```rust
// Inside eval_snapshot for Operator::Join
let w = l_weight * *r_weight;
*out.entry(l_key.clone()).or_insert(0) += w;
```

The ring structure guarantees that incrementalizing joins is algebraically sound — the delta rules for joins follow directly from the distributive law of the ring. This is not just a convenient coincidence; it is the mathematical foundation that makes the entire DBSP framework work.

*Checkpoint: A Z-set is a map from elements to integer weights. Positive weights mean present, negative mean deleted, zero means absent. Z-sets form an abelian group under addition, which means deltas compose correctly. They also form a ring with multiplication, which is what makes joins work. The Sp00ky SSP implements Z-sets as a HashMap with FxHasher, and provides both full-multiplicity and membership-only interfaces. Now that we understand the data structure, let us tackle the real problem: how do you keep a view up to date when the underlying data changes?*

---

## Part 3: Incremental View Maintenance — The Core Problem

Incremental View Maintenance (IVM) is one of the oldest problems in database research, dating back to the 1990s. The question is deceptively simple: given a materialized view defined by a query, and a change to the underlying data, how do you update the view without re-running the entire query?

### Traditional Approaches and Their Limits

The **naive approach** is to just re-run the query. When any base table changes, throw away the cached result and compute it from scratch. This is correct, simple, and catastrophically expensive. For a table with N records and a view with a filter, every single insert triggers an O(N) scan. If you have M views, the cost per mutation is O(N * M). In a reactive system where mutations happen continuously, this is untenable.

The **trigger-based approach** says: write custom logic for each view that manually tracks what changed. When a row is inserted into `thread`, check if it passes the view's filter, and if so, add it to the materialized result. This works for simple cases but breaks down quickly. What if the view involves a join? What if it has a `LIMIT` clause? What if the filter references a parameter that might change? The custom logic becomes a maintenance nightmare — you are essentially writing a hand-optimized query engine for each individual view.

The **CDC (Change Data Capture) approach** captures changes from the database's write-ahead log and replays them downstream. This is better — you get a reliable stream of mutations. But you still need per-query logic to determine how a base-table change affects each derived view. CDC gives you the *what changed* but not the *how does this affect the view*.

### The DBSP Approach

DBSP solves this systematically with a single elegant idea: decompose the query into a **circuit** of operators, and make each operator capable of two things:

1. **Snapshot evaluation**: Given the full input data, produce the full output data. This is the traditional query.
2. **Delta evaluation**: Given a *delta* (change) to the input, produce the corresponding *delta* to the output.

For many operators, the delta rule is simpler than the full evaluation. Consider a filter: `SELECT * FROM user WHERE status = 'active'`. If a new user is inserted, the delta evaluation is trivial — does this one new record have `status = 'active'`? If yes, it is in the output delta. If no, the output delta is empty. You did not touch any other record. The cost is O(1) per mutation, regardless of how many records are in the table.

For a map (or projection) operator, it is the same story: `Map(delta) = delta.map(f)`. Just apply the mapping function to each element of the delta.

The hard cases are operators that need *state*. A join between tables A and B is the canonical example. If a new record appears in table A, you need to check it against *all* of table B (not just B's delta) to find matches. This means the join operator needs to remember the accumulated state of its inputs across time steps. We will get to how DBSP handles this with the integration operator in Part 5.

### Dual-Mode Evaluation in the SSP

The Sp00ky SSP implements this dual-mode approach directly. In `packages/ssp/src/engine/view.rs`, the `compute_view_delta` method is the decision point:

```rust
fn compute_view_delta(
    &mut self,
    deltas: &FastMap<String, ZSet>,
    db: &Database,
    is_first_run: bool,
) -> ZSet {
    if is_first_run {
        // First run: full scan and diff
        self.compute_full_diff(db)
    } else {
        // Try incremental evaluation first
        if let Some(delta) = self.eval_delta_batch(&self.plan.root, deltas, db, self.params.as_ref()) {
            delta
        } else {
            // Fallback to full scan
            self.compute_full_diff(db)
        }
    }
}
```

The logic is clear: on the first run, there is no previous state to diff against, so we do a full scan. On subsequent runs, we try the incremental path (`eval_delta_batch`). If the incremental evaluator can handle the query, great — we get O(delta) performance. If it cannot (it returns `None`), we fall back to the expensive full-scan path.

Currently, `eval_delta_batch` can incrementalize `Scan` and `Filter` operators. For `Join`, `Limit`, and `Project` with subqueries, it returns `None`, triggering the fallback. This means every mutation that affects a view with a join or a limit clause triggers a full table scan, a full evaluation, and a diff against the cached state. The result is correct, but the performance is that of the naive approach for these query types.

This is the single most important thing the rebuild needs to fix: making `eval_delta_batch` return actual incremental deltas for all operator types.

*Checkpoint: IVM is the problem of keeping views up to date as data changes. Traditional approaches either re-run the whole query (expensive) or require custom per-view logic (fragile). DBSP decomposes queries into operators that can process deltas independently. The SSP implements this with a dual-mode evaluator that tries incremental first and falls back to full scan. The rebuild goal is to eliminate the fallback for all operator types. Next, let us look at how these operators are wired together.*

---

## Part 4: The Circuit Model — Computation as Wiring

In DBSP, a query is not a procedure that runs from top to bottom. It is a **circuit** — a directed graph of operators connected by streams of Z-sets. If you have ever seen an electronics circuit diagram, the analogy is precise: operators are components (resistors, capacitors), streams are wires carrying signals, and the whole thing is "clocked" — at each tick, new data flows in, propagates through the components, and produces outputs.

### Queries as Dataflow Graphs

Consider the query `SELECT * FROM user WHERE status = 'active'`. As a circuit, this is:

```
[user table deltas] --> Scan("user") --> Filter(status = 'active') --> [view output]
```

Two operators. One wire between them. At each time step (each mutation to the `user` table), the `Scan` operator produces the delta for the `user` table, the `Filter` operator checks which records in that delta pass the predicate, and the result flows out as the view's delta.

Now consider a more complex query:

```sql
SELECT *, (SELECT * FROM user WHERE id = $parent.author LIMIT 1) AS author
FROM thread
ORDER BY created_at DESC
LIMIT 10
```

This becomes a deeper circuit:

```
[thread deltas] --> Scan("thread") --> Project(*, subquery) --> Limit(10, ORDER BY created_at DESC) --> [view output]
                                              |
                                    [user deltas] --> Scan("user") --> Filter(id = $parent.author) --> Limit(1)
```

The subquery introduces a branch: the `Project` operator triggers a sub-circuit for each parent record. Changes to either the `thread` or `user` table can affect this view's output.

### The Sp00ky Circuit

In the SSP, the `Circuit` struct in `packages/ssp/src/engine/circuit.rs` is the top-level container that ties everything together:

```rust
pub struct Circuit {
    pub db: Database,
    pub views: Vec<View>,
    pub dependency_list: FastMap<TableName, DependencyList>,
}
```

The `Database` holds all the base tables. Each `Table` stores both the row data (`rows: FastMap<RowKey, Sp00kyValue>`) and the membership Z-set (`zset: ZSet`). The `views` vector holds all registered materialized views, each with its `QueryPlan` (the operator tree), a `cache: ZSet` (the current view state), and a `last_hash` (for change detection). The `dependency_list` is the routing table — it maps each table name to the indices of views that depend on that table.

When a record is ingested, the flow is:

1. **Mutate the base table**: `Table::apply_mutation` updates both the `rows` map and the `zset`, computing a Z-set key like `"user:charlie"` with weight +1 (create) or -1 (delete).

2. **Find affected views**: The circuit looks up `dependency_list["user"]` to find which views reference the `user` table. The dependency list uses `SmallVec<[ViewIndex; 4]>` — stack allocation for up to 4 dependent views, which covers the common case without heap allocation.

3. **Propagate to each view**: For each affected view, the circuit calls `process_delta` (single record) or `process_batch` (batch of records), which triggers the incremental or snapshot evaluation pipeline.

4. **Collect results**: Any `ViewUpdate` results (changed views) are collected and returned to the caller.

### The Operator Tree

The operator tree for each view is defined by the `Operator` enum in `packages/ssp/src/engine/operators/operator.rs`:

```rust
pub enum Operator {
    Scan { table: String },
    Filter { input: Box<Operator>, predicate: Predicate },
    Join { left: Box<Operator>, right: Box<Operator>, on: JoinCondition },
    Project { input: Box<Operator>, projections: Vec<Projection> },
    Limit { input: Box<Operator>, limit: usize, order_by: Option<Vec<OrderSpec>> },
}
```

This is a recursive tree structure — each operator points to its child operators via `Box<Operator>`. A `Filter` wraps a `Scan`, a `Limit` wraps a `Project` which wraps a `Filter`, and so on. The tree is built by the SQL converter in `packages/ssp/src/converter.rs`, which uses the `nom` parser combinator library to parse SurrealQL into JSON, which is then deserialized into these Rust types.

### End-to-End Trace

Let us trace a concrete example. A new user `user:charlie` is created with `{"status": "active", "name": "Charlie"}`. There is a view registered: `SELECT * FROM user WHERE status = 'active'`.

Step 1: `ingest_single` is called with a `BatchEntry::create("user", "charlie", data)`. The circuit calls `Table::apply_mutation(Operation::Create, "charlie", data)`, which inserts the row into `table.rows` and creates a Z-set entry `{"user:charlie": 1}` in `table.zset`.

Step 2: The circuit calls `ensure_dependency_list()` and looks up `dependency_list["user"]`, finding the index of our view.

Step 3: The view's `process_delta` is called with `Delta { table: "user", key: "user:charlie", weight: 1, content_changed: true }`.

Step 4: Since this is a `Filter` over a `Scan` (the `is_simple_filter` flag is true), the fast path `try_fast_single` kicks in. It checks: does `"user"` match the filter's table? Yes. Does the record pass the predicate `status = 'active'`? It looks up the row data, finds `status: "active"`, and confirms. So it calls `apply_single_create("user:charlie")`, which adds the key to the view's cache.

Step 5: The view builds a `ViewUpdate` with the delta (one addition: `"user:charlie"`), computes a new hash of the cache, confirms the hash changed from the last one, and returns the update.

Step 6: The caller receives the `ViewUpdate` and can broadcast it to connected clients.

The entire operation touched exactly one record, one view, and one predicate check. It did not scan the table. It did not re-evaluate any other record. That is the power of incremental processing.

*Checkpoint: DBSP models queries as circuits — directed graphs of operators connected by Z-set streams. The SSP implements this with a Circuit struct that routes mutations through a dependency list to affected views, which evaluate their operator trees incrementally (or fall back to snapshot). The query `SELECT * FROM user WHERE status = 'active'` becomes Scan -> Filter, and a single insertion triggers O(1) work through the fast path. But what about operators that need to remember past data? That is where the integration operator comes in.*

---

## Part 5: The Integration Operator — Memory in the Circuit

So far we have talked about operators that are *stateless* — they take an input delta and produce an output delta without needing to remember anything. `Filter(delta) = delta.filter(predicate)`. Simple. But many operators need *memory*. A join needs to remember the accumulated state of its inputs. A `DISTINCT` operator needs to remember what it has already seen. An aggregation needs running totals. How does DBSP handle state?

### The Z^{-1} Operator

The answer is the **integration operator**, written as Z^{-1} (or sometimes just I). It is the simplest operator in DBSP, and also the most important. Here is what it does:

At each time step t:
- **Input**: `delta[t]` — the change at this step.
- **Output**: `state[t-1]` — the accumulated state from all previous steps.
- **Internal update**: `state[t] = state[t-1] + delta[t]`.

That is it. The integration operator is just a running sum. It takes deltas in, stores the accumulated state, and outputs the state *from the previous step* (not the current one — this is the "delay" that prevents circular dependencies in the circuit).

The inverse of integration is **differentiation** (written D). The differentiation operator takes a stream of states and produces a stream of deltas: `delta[t] = state[t] - state[t-1]`. If integration is the running sum, differentiation is the running difference.

### Why Integration Matters

Consider a join `A JOIN B ON A.x = B.y`. When a new record arrives in table A (call it `delta_A`), you need to join it against *all* of table B — not just B's delta, but B's entire accumulated state. Where does that accumulated state come from? The integration operator on B's input stream.

The full delta rule for a join is:

```
delta_output = (delta_A JOIN state_B) + (state_A JOIN delta_B) + (delta_A JOIN delta_B)
```

Three terms. The first joins A's new records against B's history. The second joins B's new records against A's history. The third catches cases where both sides changed simultaneously. Each of `state_A` and `state_B` comes from an integration operator.

### The Incrementalization Theorem

The central theorem of DBSP is: for any query Q, the incremental version is:

```
Q_incremental = D . lift(Q) . I
```

Where `I` is integration (accumulate deltas into state), `lift(Q)` applies Q pointwise to Z-sets (treating it as a streaming operator), and `D` is differentiation (extract deltas from the output state). In plain English: integrate the input deltas into state, run the query on the state, and differentiate the output state to get output deltas.

This is the "correct but expensive" baseline — it still runs the full query at each step. The magic of DBSP is that for each specific operator (filter, join, aggregate, etc.), this formula can be *simplified* into an efficient incremental rule that avoids the full computation. The papers prove these simplifications for all standard relational operators.

### Integration in the SSP

The SSP's equivalent of the integration operator is the `cache: ZSet` field on each `View`. This stores the accumulated view output state — the set of all record keys currently in the view:

```rust
pub struct View {
    pub plan: QueryPlan,
    pub cache: ZSet,        // <-- This is the integration operator's state
    pub last_hash: String,
    // ...
}
```

When `process_batch` runs, the integration step happens in `apply_cache_delta`:

```rust
fn apply_cache_delta(&mut self, delta: &ZSet) {
    // This IS the integration operation: cache[t] = cache[t-1] + delta[t]
    self.cache.apply_membership_delta(delta);
}
```

However — and this is a critical observation — the SSP only has integration at the *view output level*. It does not maintain per-operator state. A join operator does not have its own stored state for each input. This means when the SSP encounters a join, it cannot compute the incremental delta rule (which requires `state_A` and `state_B`), so it falls back to full snapshot evaluation.

This is the key architectural gap. A proper DBSP implementation would have integration operators at each stateful node in the operator tree, not just at the output. The rebuild should introduce per-operator state to eliminate the full-snapshot fallback.

*Checkpoint: The integration operator Z^{-1} is a running sum that gives the circuit memory. It accumulates deltas into state, which stateful operators (like joins) need for incremental evaluation. The differentiation operator D is its inverse, converting state streams back to delta streams. The SSP has integration at the view level (the cache), but not at the per-operator level — which is why joins and limits currently require full re-computation. Now let us examine each operator in detail.*

---

## Part 6: Operators Deep Dive

Each operator in a DBSP circuit has two modes: snapshot evaluation (given full data, produce full result) and delta evaluation (given a change, produce the corresponding change). Let us go through each one.

### Scan

The `Scan` operator is the leaf of every operator tree. It reads from a base table.

In **snapshot mode**, it returns the table's full Z-set — the set of all records currently in the table with their weights. In the SSP, this is a zero-copy operation thanks to Rust's borrowing:

```rust
Operator::Scan { table } => {
    if let Some(tb) = db.tables.get(table) {
        Cow::Borrowed(&tb.zset)  // No copy!
    } else {
        Cow::Owned(FastMap::default())
    }
}
```

In **delta mode**, it returns the delta Z-set for this table — the mutations that happened in this time step. In the SSP's `eval_delta_batch`:

```rust
Operator::Scan { table } => {
    if let Some(d) = deltas.get(table) {
        Some(d.clone())
    } else {
        Some(FastMap::default())
    }
}
```

Simple, efficient, and fully incremental.

### Filter

The `Filter` operator applies a predicate to each record. Its incremental rule is trivially simple: `Filter(delta) = delta.filter(predicate)`. If a new record passes the filter, it is in the output. If it does not, it is discarded. You never need to look at any other record.

The SSP's delta evaluation for `Filter`:

```rust
Operator::Filter { input, predicate } => {
    let upstream_delta = self.eval_delta_batch(input, deltas, db, context)?;

    // Try SIMD fast path for numeric filters
    if let Some(config) = NumericFilterConfig::from_predicate(predicate) {
        Some(apply_numeric_filter(&upstream_delta, &config, db))
    } else {
        // Standard path
        let mut out_delta = FastMap::default();
        for (key, weight) in upstream_delta {
            if self.check_predicate(predicate, &key, db, context) {
                out_delta.insert(key, weight);
            }
        }
        Some(out_delta)
    }
}
```

Notice the optimization: for simple numeric predicates (like `level > 5`), the SSP has a SIMD-friendly fast path in `packages/ssp/src/engine/eval/filter.rs` that extracts f64 values from records and processes them in batches of 8, allowing the compiler to auto-vectorize the comparison loop.

### Project / Map

The `Map` (or `Project`) operator transforms each record's shape — selecting fields, computing derived values, or attaching subquery results. For simple projections (selecting fields), the incremental rule is trivial: `Map(delta) = delta.map(f)`. Just apply the mapping function to each delta element.

In the SSP, non-subquery projections are transparent — the `Project` operator passes through the input's Z-set unchanged, since projection only affects how data is *read from* rows (via the `get_row_value` function), not the Z-set keys themselves:

```rust
Operator::Project { input, projections } => {
    // If any projection is a subquery, fall back to snapshot
    for proj in projections {
        if let Projection::Subquery { .. } = proj {
            return None;
        }
    }
    // Otherwise, pass through
    self.eval_delta_batch(input, deltas, db, context)
}
```

Subquery projections are the exception. A subquery like `(SELECT * FROM user WHERE id = $parent.author)` is a *correlated query* — its result depends on data from the parent record. When the SSP encounters a subquery projection, it falls back to snapshot mode because tracking the dependency between parent records and subquery results incrementally requires more sophisticated state management.

### Join

The join is where things get genuinely complex. For an equi-join `A JOIN B ON A.x = B.y`, the **snapshot** evaluation is a standard hash join: build an index on one side, probe from the other. The SSP implements this clearly:

```rust
Operator::Join { left, right, on } => {
    let s_left = self.eval_snapshot(left, db, context);
    let s_right = self.eval_snapshot(right, db, context);

    // BUILD: Index the right side by join field hash
    let mut right_index: FastMap<u64, Vec<_>> = FastMap::default();
    for (r_key, r_weight) in s_right.as_ref() {
        if let Some(r_val) = self.get_row_value(r_key, db) {
            if let Some(r_field) = resolve_nested_value(Some(r_val), &on.right_field) {
                let hash = hash_sp00ky_value(r_field);
                right_index.entry(hash).or_default().push((r_key, r_weight, r_field));
            }
        }
    }

    // PROBE: Iterate left, look up in right index
    for (l_key, l_weight) in s_left.as_ref() {
        if let Some(l_val) = self.get_row_value(l_key, db) {
            if let Some(l_field) = resolve_nested_value(Some(l_val), &on.left_field) {
                let hash = hash_sp00ky_value(l_field);
                if let Some(matches) = right_index.get(&hash) {
                    for (_r_key, r_weight, r_field) in matches {
                        if compare_sp00ky_values(Some(l_field), Some(*r_field)) == Ordering::Equal {
                            let w = l_weight * *r_weight;  // Ring multiplication!
                            *out.entry(l_key.clone()).or_insert(0) += w;
                        }
                    }
                }
            }
        }
    }
    Cow::Owned(out)
}
```

The **incremental** join rule, which the SSP does NOT yet implement, is:

```
delta_output = (delta_A JOIN state_B) + (state_A JOIN delta_B) + (delta_A JOIN delta_B)
```

This requires maintaining the full accumulated state of both inputs (via integration operators). When a new record arrives in A, you join it against all of stored B. When a new record arrives in B, you join it against all of stored A. The third term handles simultaneous changes. The existing hash-join code can be reused for each of these three sub-joins — the only missing piece is the per-operator stored state.

### Limit / Top-K

`LIMIT N` (often combined with `ORDER BY`) is deceptively tricky to incrementalize. The problem: adding one record to the input might push another record *out* of the top N. Deleting one record from the top N might let a previously excluded record *in*.

The incremental approach is to maintain a sorted buffer of the top N+1 elements:

- When a new element arrives (weight > 0): Insert it into the buffer. If the buffer now has more than N elements and the new element ranks in the top N, the displaced element (previously at rank N) leaves the view — emit weight -1 for it and weight +1 for the new element.
- When an element is removed (weight < 0): Remove it from the buffer. If it was in the top N and there is an N+1th element, that element now enters the view — emit weight +1 for it.

The SSP currently falls back to full snapshot for `Limit`:

```rust
Operator::Join { .. } | Operator::Limit { .. } => None,
```

The snapshot implementation sorts all records, takes the first N, and returns them. This works but is O(M log M) per mutation where M is the total number of records passing the filter, rather than O(log N) for the incremental approach.

### Aggregate (Not Yet Implemented)

The SSP does not yet have an `Aggregate` operator, but DBSP supports incremental aggregation naturally. For `SUM`, the delta rule is: `delta_SUM = SUM(delta)`. If a new record with value 5 is added, the sum increases by 5. If a record with value 3 is deleted, the sum decreases by 3.

For `COUNT`, it is even simpler: `delta_COUNT = COUNT(delta)` where COUNT counts with weights. An insertion (weight +1) adds 1 to the count. A deletion (weight -1) subtracts 1.

For `GROUP BY`, each group maintains its own running aggregate. Deltas are routed to the appropriate group based on the grouping key.

*Checkpoint: Scan and Filter are trivially incremental — process only the delta. Project passes through for simple field selections. Join needs stored state for both inputs and a three-term delta rule. Limit needs a sorted buffer. Aggregate needs running totals per group. The SSP currently incrementalizes only Scan and Filter, falling back to snapshot for everything else. The rebuild path is clear: add per-operator state and implement the delta rules for each operator type.*

---

## Part 7: Delta Processing — Only Process What Changed

Let us step back and appreciate the performance implications of what we have been discussing. Consider a system with 100,000 records spread across 10 tables, with 50 active views watching those tables. A single record is inserted into the `user` table.

In the **naive approach**, all 50 views re-run their queries. Even if only 5 of them reference the `user` table, the system does not know that without checking. And for each of those 5 views, it scans all records in all referenced tables. Cost: catastrophic.

In the **DBSP approach** with the SSP's architecture, the flow is surgical:

First, the **dependency list** narrows the field. Only views that reference the `user` table get notified. The dependency list is a `FastMap<TableName, SmallVec<[ViewIndex; 4]>>` — a hash map from table names to view indices, where the index list uses stack allocation for up to 4 entries. Looking up which views to notify is O(1).

Second, for each affected view, **delta evaluation** processes only the changed record. For a simple `Scan + Filter` view, the work is: one predicate check on one record. O(1). Not O(N). Not O(table_size). Literally one check.

Third, the **result** is a tiny delta: "record X was added to view Y." The client does not receive the entire view state — just the change.

### The Content-Update Distinction

The SSP makes a subtle but important distinction between two types of changes:

**Membership changes** are when a record enters or leaves a view. Weight goes from <= 0 to > 0 (inserted) or from > 0 to <= 0 (deleted). These affect the *set of records* in the view.

**Content updates** are when a record's data changes but it remains in the view. For example, a user updates their profile name. Their record was in the view before, and it is still in the view after. But clients need to know the data changed so they can re-render.

The `BatchDeltas` struct in `packages/ssp/src/engine/types/batch_deltas.rs` separates these:

```rust
pub struct BatchDeltas {
    /// ZSet membership deltas (weight != 0)
    pub membership: FastMap<String, ZSet>,

    /// Keys with content changes (including weight=0 updates)
    pub content_updates: FastMap<String, Vec<SmolStr>>,
}
```

And the `Operation` enum in the SSP defines three mutation types with different characteristics:

- `Create`: weight = +1, content_changed = true. A new record enters the system.
- `Update`: weight = 0, content_changed = true. An existing record's data changed, but its presence did not.
- `Delete`: weight = -1, content_changed = false. A record leaves the system.

This separation allows the system to handle the extremely common case of "a record was updated but still matches the same views" without unnecessary membership recalculation. The view detects that the key is already in its cache, confirms it still passes the filter, and emits a content-update notification instead of an add/remove pair.

*Checkpoint: Delta processing is what makes the system fast. The dependency list narrows mutations to relevant views (O(1) lookup). Delta evaluation processes only changed records (O(1) for Scan + Filter). And the content-update distinction handles the common case of data changes without membership recalculation. The entire pipeline is designed to minimize work per mutation.*

---

## Part 8: Output Formats — How Updates Leave the System

Once a view computes its delta, it needs to communicate the change to the outside world — typically to a client rendering a UI. The SSP supports three output formats, each with different trade-offs.

### Flat Mode

The default. A `MaterializedViewUpdate` contains the view's `query_id`, a `result_hash` (blake3 hash of all sorted record IDs), and the full `result_data` (a list of all record IDs currently in the view):

```rust
pub struct MaterializedViewUpdate {
    pub query_id: String,
    pub result_hash: String,
    pub result_data: Vec<SmolStr>,
}
```

The hash serves as a cheap change-detection mechanism. If the client already has a result with the same hash, it knows nothing changed and can skip the update. The hash is computed in `compute_flat_hash`, which sorts record IDs for determinism and uses blake3 for speed. A small optimization uses `SmallVec<[&SmolStr; 16]>` for views with 16 or fewer records, keeping the sort entirely on the stack.

The downside of Flat mode: it sends the *entire* view state on every change. For a view with 10,000 records where one record was added, you are transmitting 10,001 IDs instead of 1.

### Streaming Mode

The efficient alternative. A `StreamingUpdate` contains only the records that changed:

```rust
pub struct StreamingUpdate {
    pub view_id: String,
    pub records: Vec<DeltaRecord>,
}

pub struct DeltaRecord {
    pub id: SmolStr,
    pub event: DeltaEvent,  // Created, Updated, or Deleted
}
```

For that same 10,000-record view with one addition, Streaming mode sends a single `DeltaRecord { id: "user:charlie", event: Created }`. The bandwidth savings are enormous.

The trade-off is that Streaming mode requires the client to maintain its own view state. The client receives deltas and must apply them correctly. If it misses a delta or applies one out of order, its state silently diverges. Flat mode, by contrast, is self-healing — the client can always recover by accepting the full state.

### Tree Mode

Currently a placeholder (identical to Flat). Intended for future hierarchical grouping of results.

### The Choice in the Pipeline

The output format affects the internal processing. In `process_batch`, the view builds different result data depending on the format:

```rust
let result_data = match self.format {
    ViewResultFormat::Streaming => {
        // Only include changed records
        let mut changed_keys = Vec::with_capacity(additions.len() + removals.len() + updates.len());
        changed_keys.extend(additions.iter().cloned());
        changed_keys.extend(removals.iter().cloned());
        changed_keys.extend(updates.iter().cloned());
        changed_keys
    }
    ViewResultFormat::Flat | ViewResultFormat::Tree => {
        // Need ALL records for hash computation
        self.build_result_data()
    }
};
```

For Flat/Tree modes, `build_result_data()` collects all keys from the cache and sorts them — an O(N log N) operation where N is the view size. For Streaming mode, it only collects the changed keys — an O(delta) operation. This is why Streaming mode should be preferred for large views with small, frequent changes.

*Checkpoint: The SSP offers three output formats. Flat mode sends the full view state with a hash for change detection — simple and self-healing but bandwidth-heavy. Streaming mode sends only deltas (Created/Updated/Deleted events) — efficient but requires client-side state management. The choice of format affects the internal processing cost, with Streaming being O(delta) and Flat being O(view_size).*

---

## Part 9: The Rust Ecosystem for DBSP

Before rebuilding the SSP, it is worth surveying what already exists in the Rust ecosystem. Can we use an existing library instead of building from scratch?

### Feldera (the official DBSP implementation)

Feldera, the company founded by DBSP's creators, maintains an open-source Rust implementation at `github.com/feldera/feldera`. The `dbsp` crate within that repo is the most complete and correct DBSP implementation in existence. It supports all relational operators (including aggregation, windowing, and recursive queries), has per-operator state management, and includes a SQL-to-DBSP compiler.

If you are building a server-side streaming SQL engine, Feldera is the gold standard. Study its architecture, its operator implementations, and its testing strategies. The codebase is the best reference for understanding how DBSP theory maps to production Rust code.

However, Feldera is designed for server-side use. It has heavy dependencies (its own runtime, Tokio, various system libraries) and assumes a multi-threaded environment. It does NOT compile to WebAssembly. For the Sp00ky SSP, which must run in the browser, Feldera cannot be used directly.

### Differential Dataflow and Timely Dataflow

Frank McSherry's `differential-dataflow`, built on top of `timely-dataflow`, is the other major Rust implementation of incremental computation on collections. It predates DBSP but shares many of the same ideas. Differential dataflow operates on *arrangements* — indexed collections with logical timestamps — and supports incremental joins, aggregation, and even iterative computation (which DBSP also supports but the SSP does not need).

Materialize, the streaming SQL database, built its entire product on differential-dataflow, proving its production viability at scale. The codebase is well-tested and well-documented.

Like Feldera, `timely-dataflow` depends on threading and networking primitives that do not compile to WASM. It is not suitable for the browser. But its design patterns — particularly how it manages per-operator state and how it handles join incrementalization — are worth studying.

### The WASM Constraint

This is the fundamental constraint that shapes the SSP's architecture. The SSP must run in the browser as WebAssembly. This means:

- No threads (WASM is single-threaded in most browser environments).
- No filesystem access.
- No networking primitives (no TCP sockets, no NATS clients).
- Limited memory (browsers typically cap WASM memory at 2-4 GB).
- No system allocator (the SSP uses mimalloc on native and the JS allocator on WASM, with conditional compilation).

The SSP already handles this with `cfg` attributes:

```rust
#[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
// Use rayon for parallel batch processing

#[cfg(any(target_arch = "wasm32", not(feature = "parallel")))]
// Sequential fallback for browser
```

This means a custom implementation is the right choice for the WASM target. The existing libraries are too heavy and too server-oriented. But the SSP should be *informed by* those libraries' design patterns.

### Other Notable Libraries

**salsa** is a Rust incremental computation framework used by rust-analyzer (the Rust language server). It is designed for compiler-like workloads with fine-grained dependency tracking. Not suitable for relational queries, but interesting for its memoization and dependency graph design.

**arroyo** is a Rust streaming SQL engine, but it is server-side only and designed for event stream processing (think Kafka), not database view maintenance.

### Trade-Off Summary

| Approach | WASM Compatible | Feature Complete | Maintenance Burden | Learning Curve |
|---|---|---|---|---|
| Use Feldera directly | No | Yes | Low | Medium |
| Fork Feldera, strip for WASM | Unlikely | Partial | Very High | High |
| Use differential-dataflow | No | Yes | Low | High |
| Build custom, inspired by Feldera | Yes | You control | Medium | Medium |

The recommendation is clear: **build custom, informed by Feldera's design patterns**. Study Feldera's operator implementations and state management. Port the concepts, not the code. Keep the SSP lightweight enough for WASM while implementing the full DBSP algebra.

*Checkpoint: The Rust ecosystem has two excellent DBSP/dataflow libraries (Feldera and differential-dataflow), but neither compiles to WASM. The SSP must be a custom implementation. Study Feldera for design patterns — especially per-operator state and join incrementalization — but build for the browser's constraints.*

---

## Part 10: Rebuilding the SSP — Practical Recommendations

Based on everything we have covered, here are concrete recommendations for rebuilding the SSP cleanly.

### Principle 1: Separate the Algebra from the Storage

The Z-set operations (add, diff, membership) should be a standalone, thoroughly tested module with no dependencies on the rest of the engine. The current `zset.rs` is a good foundation but mixes two models (full multiplicity and membership). The rebuild should:

- Keep the full `ZSetOps` (add_delta, diff) as the core algebra.
- Layer `ZSetMembershipOps` on top as a normalization strategy, not a parallel implementation.
- Add property-based tests (using a crate like `proptest`) to verify the group axioms: `a + (-a) = 0`, `a + b = b + a`, `(a + b) + c = a + (b + c)`.

### Principle 2: Per-Operator State, Not Per-View State

This is the single most impactful architectural change. Currently, the SSP stores state only at the view output level (the `cache: ZSet` on each `View`). A proper DBSP implementation stores state at each stateful operator.

Concretely, this means changing the `Operator` enum from a pure data description to a stateful struct:

```rust
// Current: pure description, no state
pub enum Operator {
    Scan { table: String },
    Filter { input: Box<Operator>, predicate: Predicate },
    Join { left: Box<Operator>, right: Box<Operator>, on: JoinCondition },
    // ...
}

// Proposed: each operator holds its integration state
pub enum OperatorNode {
    Scan { table: String },
    Filter { input: Box<OperatorNode>, predicate: Predicate },
    Join {
        left: Box<OperatorNode>,
        right: Box<OperatorNode>,
        on: JoinCondition,
        // NEW: stored state for incremental join
        left_state: ZSet,
        right_state: ZSet,
    },
    Limit {
        input: Box<OperatorNode>,
        limit: usize,
        order_by: Option<Vec<OrderSpec>>,
        // NEW: sorted buffer for incremental top-K
        buffer: BTreeMap<SortKey, SmolStr>,
    },
    // ...
}
```

With per-operator state, `eval_delta_batch` never needs to return `None`. Every operator can process its delta using its stored state, and the full-snapshot fallback can be eliminated entirely (except for the initial load).

### Principle 3: Typed Operator Graph

The current converter produces JSON (`serde_json::Value`) which is then deserialized into the `Operator` enum. Consider building the operator tree directly in Rust types. The converter should output `OperatorNode` directly, not an intermediate JSON representation. This eliminates a serialization round-trip and makes the code easier to reason about.

### Principle 4: Clear Delta/State Type Separation

Currently, the same `ZSet` type is used for both deltas (changes) and accumulated states. While algebraically they are the same type, the semantics are very different. A delta with `{"user:alice": -1}` means "alice was deleted." A state with `{"user:alice": -1}` would be nonsensical (you cannot have negative presence).

Consider introducing wrapper types:

```rust
pub struct Delta(pub ZSet);  // Changes — negative weights are deletions
pub struct State(pub ZSet);  // Accumulated state — weights should be >= 0
```

This makes the code self-documenting and prevents accidental misuse (like adding two states together, which is not meaningful).

### Incrementalizing Joins

The join delta rule:

```
delta_output = (delta_A JOIN state_B) + (state_A JOIN delta_B) + (delta_A JOIN delta_B)
```

Implementation steps:
1. Add `left_state: ZSet` and `right_state: ZSet` fields to the `Join` operator.
2. On each step, before computing the output delta, update the states: `left_state += delta_left`, `right_state += delta_right`.
3. Compute the three join terms using the existing hash-join code.
4. Sum the three terms to produce the output delta.
5. The existing hash-join implementation in `eval_snapshot` can be refactored into a reusable `hash_join(left: &ZSet, right: &ZSet, condition: &JoinCondition, db: &Database) -> ZSet` function.

### Incrementalizing Limit / Top-K

Implementation steps:
1. Add a `BTreeMap<SortKey, SmolStr>` buffer to the `Limit` operator, maintaining the top N+1 elements.
2. When a delta arrives with weight > 0: insert into the buffer. If the buffer exceeds N and the new element is in the top N, emit +1 for it and -1 for the displaced element.
3. When a delta arrives with weight < 0: remove from the buffer. If it was in the top N and there is a new N+1th element, emit +1 for the new entrant.
4. For `ORDER BY` with multiple fields, the sort key should be a tuple of the field values.

### Handling Subqueries

Subqueries like `(SELECT * FROM user WHERE id = $parent.author)` are correlated queries — their result depends on the parent record's data. In DBSP terms, these are **lateral joins** or **dependent joins**.

The approach:
1. Track a dependency map: `parent_key -> set of subquery result keys`.
2. When a parent record changes, re-evaluate its subqueries and diff the results.
3. When a subquery's base data changes, find all parent records that reference it and check if their subquery results changed.

The SSP's `expand_with_subqueries` method already does step 1 and 2 in snapshot mode. The task is to maintain the dependency map incrementally.

### Phased Rebuild Plan

**Phase 1 — Algebraic Foundation**: Clean up `zset.rs`. Decide on multiplicity vs. membership model (recommend: multiplicity core with membership layer). Add property-based tests. Introduce `Delta` and `State` wrapper types.

**Phase 2 — Per-Operator State**: Refactor `Operator` into `OperatorNode` with per-operator state fields. Refactor `eval_delta_batch` to use the stored state. Eliminate the snapshot fallback for Scan and Filter (should already work).

**Phase 3 — Incremental Join**: Implement the three-term join delta rule. Add integration operators for both join inputs. Test with existing join queries.

**Phase 4 — Incremental Limit/Top-K**: Implement the sorted-buffer approach. Test with `ORDER BY ... LIMIT N` queries.

**Phase 5 — Aggregation**: Add an `Aggregate` operator with incremental rules for SUM, COUNT, AVG, MIN, MAX, and GROUP BY.

**Phase 6 — Benchmark and Optimize**: Use the existing benchmark test in `packages/ssp/tests/real_worl_benchmark.rs` as a baseline. Measure the performance improvement from eliminating snapshot fallbacks.

*Checkpoint: The rebuild should focus on four principles: clean algebra, per-operator state, typed operator graphs, and clear delta/state separation. The most impactful change is per-operator state, which eliminates the snapshot fallback for joins and limits. The phased plan goes from foundations to optimization.*

---

## Part 11: Closing — From Theory to Practice

Let us bring it all together.

DBSP is a mathematical framework that turns any SQL query into an incremental computation. The core data structure is the Z-set — a map from elements to integer weights, where positive means present and negative means deleted. Z-sets form an abelian group under addition, which means deltas compose correctly. They form a ring with multiplication, which makes joins work.

The computation is modeled as a circuit — a directed graph of operators (Scan, Filter, Join, Project, Limit, Aggregate) connected by streams of Z-sets. Each operator has a snapshot mode (full computation) and a delta mode (incremental computation). The integration operator Z^{-1} gives the circuit memory, allowing stateful operators like joins to access accumulated state from previous time steps.

The Sp00ky Stream Processor is a working implementation that gets the fundamentals right. It has Z-sets, delta propagation, a dependency list for efficient routing, fast paths for simple Scan + Filter queries, and multiple output formats (Flat with hash-based deduplication, Streaming with per-record delta events). But it currently falls back to full snapshot evaluation for complex operators — joins, limits, and subquery projections.

The rebuild path is clear. Add per-operator state (integration operators at each stateful node). Implement the join delta rule (three-term: delta-A with state-B, state-A with delta-B, delta-A with delta-B). Implement incremental top-K (sorted buffer). Add aggregation operators with incremental rules. The Rust ecosystem has excellent references in Feldera and differential-dataflow, but neither compiles to WASM, so the SSP must remain a custom implementation informed by their design patterns.

The end result will be a DBSP engine that runs in the browser and on the server, processing record changes in O(delta) time for any query type, and emitting minimal updates to keep every connected client's view perfectly synchronized with the database.

### Further Reading

- **"DBSP: Automatic Incremental View Maintenance"** — Budiu, Ryzhyk, et al. (VLDB 2023). The foundational paper. Proves that any query can be incrementalized.
- **Feldera Blog Series** — `feldera.com/blog`. Accessible explanations of DBSP concepts with examples.
- **Feldera Source Code** — `github.com/feldera/feldera`. The reference Rust implementation. Study the operator implementations in the `dbsp` crate.
- **Frank McSherry's Blog** — `github.com/frankmcsherry/blog`. Deep technical posts on differential dataflow, which shares many ideas with DBSP.
- **differential-dataflow Documentation** — `docs.rs/differential-dataflow`. The other major Rust incremental computation library.
- **"Naiad: A Timely Dataflow System"** — Murray, McSherry, et al. (SOSP 2013). The foundational paper for timely dataflow, which underpins differential-dataflow.

---

## Appendix A: Glossary

| Term | Definition |
|---|---|
| **Z-set** | A function from a domain D to the integers Z. A generalized multiset where elements can have negative weights. The core data structure of DBSP. |
| **Weight** | The integer value associated with an element in a Z-set. Positive = present, negative = deleted, zero = absent. |
| **Delta** | A Z-set representing a *change* to a collection. Positive weights are insertions, negative weights are deletions. |
| **State** | A Z-set representing the *accumulated* contents of a collection at a point in time. Weights should be non-negative. |
| **Circuit** | A directed graph of operators connected by Z-set streams. The DBSP model of a query. |
| **Operator** | A node in the circuit that transforms one or more input Z-set streams into an output Z-set stream. Examples: Scan, Filter, Join, Limit. |
| **Integration (Z^{-1})** | The operator that accumulates deltas into state. At each step: `state[t] = state[t-1] + delta[t]`. Provides memory to the circuit. |
| **Differentiation (D)** | The inverse of integration. Computes deltas from states: `delta[t] = state[t] - state[t-1]`. |
| **IVM** | Incremental View Maintenance. The problem of updating a materialized view when the underlying data changes, without re-running the entire query. |
| **DBSP** | Database Stream Processing. The mathematical framework for IVM developed by Budiu, McSherry, et al. Proves that any relational query can be incrementalized. |
| **Snapshot Evaluation** | Running a query on the full dataset to produce the full result. The traditional (non-incremental) approach. |
| **Delta Evaluation** | Processing only the *change* to the input and producing the corresponding *change* to the output. The incremental approach. |
| **Materialized View** | A cached query result that is kept up to date as the underlying data changes. |
| **Membership Model** | A simplified Z-set model where weights are normalized to 0 (absent) or 1 (present). Sufficient for tracking which records are in a view, but insufficient for aggregation. |
| **Multiplicity Model** | The full Z-set model where weights can be any integer. Required for correct aggregation (COUNT, SUM, etc.) and duplicate handling. |
| **Dependency List** | A map from table names to the set of views that reference that table. Used for efficient routing of mutations to affected views. |
| **SSP** | Sp00ky Stream Processor. The Rust crate implementing the DBSP engine for the Sp00ky framework. |

## Appendix B: Delta Rules Quick Reference

| Operator | Delta Rule | State Required? | SSP Status |
|---|---|---|---|
| **Scan** | `delta_out = delta_table` | No | Implemented |
| **Filter** | `delta_out = delta_in.filter(predicate)` | No | Implemented |
| **Map / Project** | `delta_out = delta_in.map(f)` | No | Implemented (non-subquery) |
| **Union** | `delta_out = delta_A + delta_B` | No | Not needed yet |
| **Join (A JOIN B)** | `delta_out = (delta_A JOIN state_B) + (state_A JOIN delta_B) + (delta_A JOIN delta_B)` | Yes (both sides) | NOT implemented (falls back to snapshot) |
| **Distinct** | `delta_out = D(distinct(I(delta_in)))` | Yes | Not needed yet |
| **Limit / Top-K** | Maintain sorted buffer of K+1 elements; emit displacements | Yes (sorted buffer) | NOT implemented (falls back to snapshot) |
| **Aggregate (SUM)** | `delta_SUM = SUM(delta_in)` (weighted) | Yes (running total) | NOT implemented |
| **Aggregate (COUNT)** | `delta_COUNT = sum of weights in delta_in` | Yes (running count) | NOT implemented |
| **Aggregate (GROUP BY)** | Route deltas to groups, apply per-group aggregate rule | Yes (per-group state) | NOT implemented |
| **Subquery (lateral)** | Track parent-to-result dependency map; re-evaluate on parent or base data change | Yes (dependency map) | Falls back to snapshot |

## Appendix C: Key Files in the Sp00ky SSP Codebase

| File | Purpose |
|---|---|
| `packages/ssp/src/engine/types/zset.rs` | Z-set type definitions, `ZSetOps` trait, `ZSetMembershipOps` trait, `WeightTransition` enum |
| `packages/ssp/src/engine/circuit.rs` | Circuit struct, `ingest_single`, `ingest_batch`, `propagate_deltas`, `register_view`, dependency list |
| `packages/ssp/src/engine/view.rs` | View struct, `process_delta`, `process_batch`, `eval_delta_batch`, `eval_snapshot`, `compute_view_delta`, `apply_cache_delta` |
| `packages/ssp/src/engine/operators/operator.rs` | Operator enum (Scan, Filter, Join, Project, Limit), `referenced_tables` |
| `packages/ssp/src/engine/operators/predicate.rs` | Predicate types (Eq, Neq, Gt, Lt, And, Or, Prefix) |
| `packages/ssp/src/engine/operators/projection.rs` | Projection types (Field, All, Subquery), JoinCondition, OrderSpec |
| `packages/ssp/src/engine/update.rs` | ViewUpdate, MaterializedViewUpdate, StreamingUpdate, DeltaEvent, `build_update`, `compute_flat_hash` |
| `packages/ssp/src/engine/types/batch_deltas.rs` | BatchDeltas struct separating membership and content changes |
| `packages/ssp/src/engine/eval/filter.rs` | SIMD-friendly numeric filter fast path |
| `packages/ssp/src/converter.rs` | SurrealQL to operator tree parser (nom-based) |
| `packages/ssp/src/service.rs` | Service layer for ingestion and view registration |
| `packages/ssp/Cargo.toml` | Dependencies and WASM/native conditional compilation |
