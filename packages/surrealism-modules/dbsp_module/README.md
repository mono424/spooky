# DBSP Incremental View Module

**Package**: `spooky/dbsp_module`
**Version**: `0.1.0`

This module implements a **stateless, in-memory Incremental View Maintenance (IVM)** engine for SurrealDB using WebAssembly (WASM). It is inspired by the principles of **DBSP (Digital Bitstream Signal Processing)**, treating database changes as a stream of signals (deltas) that flow through a circuit of views to update materialized results efficiently.

## Core Concepts

### 1. Z-Sets (Zero-Sets)

Traditional databases treat tables as sets of records. In DBSP, collections are represented as **Z-Sets**: a mapping of `Data -> Weight`.

- **Weight `+1`**: Integration (Insert).
- **Weight `-1`**: Differentiation (Delete).
- **Weight `0`**: The item does not exist in the set.

This allows for efficient algebraic manipulation of sets. An update is simply the addition of two deltas: `{ OldData: -1 }` + `{ NewData: +1 }`.

### 2. Stateless Architecture (The "Circuit")

Due to the stateless nature of SurrealDB's WASM runtime (memory is reset between function calls), the entire "Circuit" state is serialized and persisted in the database (`_spooky_module_state` table).

1.  **Load**: The current `Circuit` JSON is loaded from SurrealDB into the WASM module.
2.  **Process**: The module applies the incoming Delta (Change) to the circuit.
3.  **Step**: The change propagates through registered Views.
4.  **Save**: The updated `Circuit` state is returned and saved back to SurrealDB.

### 3. Incremental Views

A **View** continuously maintains the result of a query without re-running the full query on every change.

- **Cache**: Each view maintains its own Z-Set cache of matched records.
- **Processing**: When a mutation occurs (`ingest`), the view checks if the _delta_ is relevant (e.g., matches the source table and filter).
- **Update**: If relevant, it applies the weight to its internal cache. A view only emits an update event if the resulting set of IDs changes.

### 4. ID Trees (Radix/Merkle Trees)

To facilitate efficient synchronization with clients, the resulting list of record IDs is structured into a **Radix Tree** (specifically, a deterministic hash tree).

- **Leaves**: Sorted lists of Record IDs.
- **Nodes**: Hashes representing the content of their children.
- **Root**: A single hash representing the entire state of the view.

This allows clients to compare their local state hash with the server's root hash. If they match, no data verification is needed. If they differ, they can traverse the tree to find exactly which "page" of IDs is out of sync.

## API Reference

These functions are exported via the generic `fn::surrealism::runtime` wrapper but are defined internally as:

### `register_query`

Registers a new live query (incantation) into the circuit.

**Arguments:**

- `id` (String): Unique identifier for the query (e.g., `inc_active_threads`).
- `plan_json` (String): JSON object defining the query plan (Source Table, Filter Prefix).
- `state` (Value): The current serialized circuit state.

**Returns:**

- `new_state`: The updated circuit.

---

### `unregister_query`

Removes a query from the circuit, freeing up its memory in the state.

**Arguments:**

- `id` (String): The query ID to remove.
- `state` (Value): The current state.

---

### `ingest`

The main entry point for data mutations. It accepts a change event from SurrealDB and propagates it through the circuit.

**Arguments:**

- `table` (String): The table being modified (e.g., `thread`).
- `operation` (String): `CREATE`, `UPDATE`, or `DELETE`.
- `id` (String): The Record ID text (e.g., `thread:123`).
- `record` (Value): The full record content (for creating the Delta).
- `state` (Value): The current state.

**Returns:**

- `updates`: Array of `MaterializedViewUpdate` events for any views that changed.
- `new_state`: The updated circuit state.

## Internal Data Structures

- **`Database`**: Holds the "Ground Truth" state of all tracked tables (Weights + Data).
- **`View`**: Contains the `QueryPlan` (Operator Tree) and the active Cache (Z-Set).
- **`Operator` Tree**:
  The query plan is now a recursive tree of operators, defined in JSON:

  ```json
  {
    "op": "filter",
    "predicate": { "type": "prefix", "prefix": "thread:active" },
    "input": {
      "op": "join",
      "on": { "left_field": "author", "right_field": "id" },
      "left": { "op": "scan", "table": "thread" },
      "right": { "op": "scan", "table": "user" }
    }
  }
  ```

  **Supported Operators:**

  - `scan`: Reads a table.
  - `filter`: Applies a predicate (`prefix` or `eq`).
  - `join`: Inner join (nested loop).
  - `limit`: Top-N (requires sorting by ID).

## Limitations

- **Memory**: The entire state of all active views is held in a single JSON object. This is not suitable for millions of active views or extremely large datasets per view within this specific WASM implementation.
- **Persistence**: State serialization cost increases with the size of the cached views.
