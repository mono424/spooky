# Spooky Stream Processor Module Contracts

This document defines the interface and call contracts for the different modules interacting with the Spooky Stream Processor core logic.

## 1. Core Logic (`ssp` Rust Crate)

The shared business logic used by all other modules.

### Service Primitives

**Ingest Preparation**

```rust
fn prepare(record: Value) -> (CleanRecord, Hash);
```

- Normalizes the input record.
- Calculates the stable hash.

**View Registration Preparation**

```rust
fn prepare_registration(config: Value) -> Result<RegistrationData, Error>;
```

- Validates configuration.
- Parses options (id, sql, params, etc.).
- Returns prepared query plan and safe parameters.

### Circuit Interface (Internal)

**Ingest Record**

```rust
fn ingest_record(
  table: &str,
  op: &str,
  id: &str,
  record: Record,
  hash: &str
) -> Vec<MaterializedViewUpdate>
```

**Register View**

```rust
fn register_view(
  plan: QueryPlan,
  params: BTreeMap<String, Value>
) -> Option<MaterializedViewUpdate>
```

---

## 2. Stream Processor WASM (Client-Side)

**Caller:** TypeScript Application (via `StreamProcessorService`)
**Execution Config:** Browser / Client
**Event Trigger:** Client-side user actions or subscriptions.

### Exposed Functions

#### `ingest`

Ingests a record change into the local processor.

| Param    | Type      | Description                              |
| :------- | :-------- | :--------------------------------------- |
| `table`  | `String`  | Table name (e.g., `user`)                |
| `op`     | `String`  | Operation (`CREATE`, `UPDATE`, `DELETE`) |
| `id`     | `String`  | Record ID (e.g., `user:123`)             |
| `record` | `JsValue` | The full record object                   |

**Returns:** `WasmStreamUpdate[]`

#### `register_view`

Registers a new active query (incantation).

| Param    | Type                    | Description          |
| :------- | :---------------------- | :------------------- |
| `config` | `WasmIncantationConfig` | Configuration object |

**Config Object Schema:**

```typescript
interface WasmIncantationConfig {
  id: string; // Incantation ID
  sql: string; // The query string
  params?: Record<string, any>; // Query parameters
  clientId: string; // Client identifier
  ttl: string; // Time to live
  lastActiveAt: string; // ISO timestamp
}
```

**Returns:** `WasmStreamUpdate` (Initial result)

#### `unregister_view`

Removes an incantation.

| Param | Type     | Description    |
| :---- | :------- | :------------- |
| `id`  | `String` | Incantation ID |

---

## 3. Spooky Sidecar (Server-Side)

**Caller:** External Services or Event Hooks (HTTP)
**Execution Config:** Standalone Service (Docker/Process)
**Event Trigger:** HTTP Requests

### HTTP Endpoints

#### `POST /ingest`

Triggers an ingest operation on the server-side circuit.

**Request Body:**

```json
{
  "table": "string",
  "op": "string",
  "id": "string",
  "record": { ... }, // Arbitrary JSON
  "_hash": "string"  // Optional: associated hash
}
```

**Response:** `200 OK`
_Side Effects:_ Updates persistent state and `_spooky_query` records in SurrealDB.

#### `POST /view/register`

Registers an incantation on the sidecar.

**Request Body:**

```json
{
  "id": "string",
  "sql": "string",
  "params": { ... },
  "clientId": "string",
  "ttl": "string",
  "lastActiveAt": "string"
}
```

**Response:** `200 OK`
_Side Effects:_ Upserts `_spooky_query` metadata record in SurrealDB.

#### `POST /view/unregister`

**Request Body:**

```json
{
  "id": "string"
}
```

---

## 4. Surrealism DBSP Module (Embedded)

**Caller:** SurrealDB Event System
**Execution Config:** Embedded Module (loaded via `.so` / `.dylib`)
**Event Trigger:** SurrealDB `DEFINE EVENT` triggers.

### sql Functions

#### `function::dbsp::ingest`

Called automatically by database events to keep the processor in sync with DB writes.

```sql
function::dbsp::ingest($table, $event, $id, $after)
```

| Arg      | Description                                              |
| :------- | :------------------------------------------------------- |
| `$table` | The table name                                           |
| `$event` | The event type (`CREATE`, `UPDATE`, `DELETE`)            |
| `$id`    | The record ID                                            |
| `$after` | The record content (use `$before` for deletes if needed) |

**Updates:** Directly modifies `_spooky_query` tables via internal Rust calls.

#### `function::dbsp::register_view`

Registers a view directly from SQL.

```sql
function::dbsp::register_view({
    "id": $id,
    "sql": "SELECT ...",
    "params": $params,
    ...
})
```

#### `function::dbsp::unregister_view`

```sql
function::dbsp::unregister_view($incantation_id)
```

---

## Summary of Interaction

| Feature     | WASM (Client)         | Sidecar (Service)        | Surrealism (Embedded)    |
| :---------- | :-------------------- | :----------------------- | :----------------------- |
| **State**   | Local (IndexedDB/Mem) | Shared (Persistent JSON) | Shared (Persistent JSON) |
| **Input**   | JS Calls              | HTTP JSON                | SQL Function Args        |
| **Output**  | JS Objects            | DB Writes                | DB Writes (Direct)       |
| **Latency** | Immediate             | Network RTT              | Zero-copy / Internal     |
