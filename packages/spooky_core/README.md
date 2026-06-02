# spooky_core

Pure-Dart port of `@spooky-sync/core`. Framework-agnostic: query subscriptions
are exposed as Dart `Stream`s, so a Flutter app consumes them with a
`StreamBuilder`.

It mirrors the JavaScript core module-for-module, with three platform-bound
pieces handled the Dart way:

- **Materialization** runs client-side through Dart FFI into the same Rust
  `ssp` DBSP circuit the browser uses (see `packages/ssp-ffi`), instead of
  WASM.
- **Local storage** is sqlite (`package:sqlite3`), replacing the embedded
  SurrealDB WASM engine. Records are stored document-style as JSON.
- **CRDT** (`openCrdtField`) is deferred; the wiring seam is left in place.

## Architecture

```
Sp00kyClient
├── LocalDatabaseService      (sqlite: records + _00_query + outbox + state)
├── StreamProcessorService    (FFI -> Rust ssp circuit; permission-seeded)
├── CacheModule               (local write + DBSP ingest bridge)
└── DataModule                (query registration, subscriptions, mutations)
        └── exposes Stream<List<Map<String,dynamic>>> via Sp00kyClient
```

The read path: register a query -> the FFI circuit returns a `localArray`
(`[id, version]` pairs) -> records are resolved from sqlite by id -> emitted on
the query's `Stream`. Mutations write locally (optimistic), ingest into the
circuit, and the resulting stream update reaches subscribers.

## Native library

The FFI processor needs `libssp_ffi`. Build and stage it with:

```bash
bash packages/ssp-ffi/build-native.sh   # -> packages/spooky_core/native/<platform>/
```

`SSP_FFI_PATH` overrides the library location (used in dev/tests).

## Usage

```dart
final client = Sp00kyClient(Sp00kyConfig(
  database: const DatabaseConfig(namespace: 'app', database: 'app'),
  schema: schema,           // { table: { columns: { name: ColumnSchema(...) } } }
  schemaSurql: schemaSurql, // DEFINE TABLE ... PERMISSIONS ...
));
await client.init();

final stream = await client.queryStream('SELECT * FROM thread', {});
stream.listen((records) => print(records)); // or StreamBuilder in Flutter

await client.create('thread:abc', {'title': 'hello'});
```

## Deliberate divergences from the JS core

1. **Permission seeding.** The browser circuit is effectively permissive; the
   native circuit is default-deny. `StreamProcessorService.seedPermissionsFromSchema`
   extracts each table's `PERMISSIONS FOR select` from `schemaSurql` and seeds
   the circuit (the way the SSP server does at boot). This is required for
   `registerView` to succeed.
2. **Local read path.** sqlite can't run SurrealQL, so query results are
   materialized from the circuit's `localArray` via `getById`, rather than by
   re-running the registered SURQL. Ordering/limit/projection stay the circuit's
   responsibility.
3. **Stream `immediate` default.** `subscribeStream` replays the current result
   set on first listen so a `StreamBuilder` renders immediately.

## Status

Implemented and tested (54 tests):
- ssp-ffi C ABI + Dart FFI bindings (round-trip against the real native lib)
- Foundations: RecordId, durations, surql builders, parser, EventSystem
- Services: sqlite local store, stream-processor service, persistence
- Modules: CacheModule, DataModule, Sp00kyClient (local-first read/write,
  reactive Streams, optimistic mutations, pending-mutation outbox)
- Remote sync: custom SurrealDB WebSocket JSON-RPC client, `Sp00kySync`
  (UpQueue/DownQueue/SyncEngine/SyncScheduler), LIVE subscription + poll
  fallback on `_00_list_ref[_user_<id>]`, reconnect re-registration.
- `AuthService` and the auth-driven session/sync wiring in `init`.
- sqlite-backed persistence (`SqlitePersistenceClient`, the default): the DBSP
  circuit state and auth token survive restarts with a file-backed store.
- TTL heartbeat lifecycle: each query re-registers at ~90% of its TTL so the
  server-side registration does not expire.
- `LocalMigrator`: schema-hash provisioning that wipes stale local data on a
  schema change (preserving the auth token).
- `run()` backend job outbox and `bucket()` file storage (`BucketHandle`:
  put/get/delete/exists/head/copy/rename/list).
- Fluent `QueryBuilder` (`client.query('table').where(...).orderBy(...).limit(...)
  .stream()`) compiling to the same SURQL shape as the JS builder.
- Codegen (`package:spooky_core/codegen.dart` + `dart run spooky_core:spooky_gen
  <schema.surql> [out.dart]`): parses `DEFINE TABLE`/`DEFINE FIELD` and emits the
  `ColumnSchema` map (for `Sp00kyConfig.schema`) plus typed model classes.

The full sync orchestration (up-queue -> remote, down-queue register + initial
fetch, LIVE -> down-sync -> Stream) is verified end-to-end against a fake
remote client. The `WebSocketSurrealClient` wire protocol is also validated
against **live SurrealDB v2.1.4 and v3.1.2** (connect/signin/use, query with
RecordId bind vars, LIVE + KILL, lifecycle), and the end-to-end mutation
up-path (local create/update -> remote) is verified through the real client on
both versions. The **down-path** (`fn::query::register` -> ssp materialization ->
`_00_list_ref_user_<id>` -> LIVE/fetch -> Stream) is also validated end-to-end
against a running ssp + SurrealDB stack, including a root-side UPDATE
propagating via the LIVE feed (see `test/integration/downpath_e2e_test.dart`,
point `SURREAL_DEV_WS` at the ssp's SurrealDB).

### Running tests

The `ssp-ffi` Rust cdylib installs `std`'s signal/stack-overflow handlers when
loaded, which the Dart `test` runner's *secondary* suite isolates don't
tolerate (a process hosts many suites). So run each test file in its own
process:

```bash
tool/run_tests.sh                 # unit tests, one process per file
tool/run_tests.sh --integration   # integration tests (need a server)
```

`dart test <single_file>` also works; the aggregate `dart test` (all files in
one process) is the only thing affected. Real single-process app usage is fine.

### Integration tests

Tagged `integration` and skipped unless a server is reachable. To run:

```bash
docker run -d --name surreal -p 18011:8000 \
  surrealdb/surrealdb:v2.1.4 start --user root --pass root --allow-all memory
cd packages/spooky_core
SURREAL_IT_ENDPOINT=ws://127.0.0.1:18011 dart test --tags integration
```

Validated against SurrealDB v2.1.4 and v3.1.2.

Not yet implemented (seam in place):
- CRDT collaborative fields (`openCrdtField`).

## Tests

```bash
dart test
```
