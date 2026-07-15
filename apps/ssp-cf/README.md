# ssp-cf — Cloudflare Durable Object shell for the SSP node

The reference **edge** host: the same `ssp_node::SspNode` / `Runtime` as the VM
(`apps/ssp`) and the local reference host (`apps/ssp-portable`), driven by a
Durable Object. Proof that "run anywhere" is an adapter swap, not a fork.

## Shape

| Concern      | Port              | This host's adapter                     |
|--------------|-------------------|-----------------------------------------|
| Ingress      | (push)            | DO `fetch(req)` → `SspNode::route`       |
| DB           | `Db`              | `ssp_node::HttpSqlDb` over `CfHttp`      |
| HTTP out     | `HttpClient`      | `CfHttp` (Workers `fetch`)              |
| Timers       | `Scheduler`       | `CfScheduler` (DO `alarm()` + `TimerMux`)|
| Persistence  | `CircuitStore`    | `CfCircuitStore` (DO storage)            |
| Spawn        | `Spawner`         | `CfSpawner` (`spawn_local`)              |
| Telemetry    | `Telemetry`       | `NoopTelemetry`                          |

The **DB stays external** (SurrealDB Cloud): `HttpSqlDb` reaches it over HTTP-RPC
via `fetch`, so this host links no surrealdb SDK and needs no SurrealDB-in-wasm.
Cold start / eviction goes through `Runtime::bootstrap` (restore from DO storage
+ `_00_rv` catch-up, else full rebuild); the DO checkpoints the circuit
periodically so an evicted instance restarts warm.

## Status

Compiles for `wasm32-unknown-unknown` (`cargo check`) against the real `worker`
0.4 API. NOT yet deployed/run: needs `worker-build` + `wrangler` (not installed
in the dev sandbox), a real SurrealDB Cloud instance, and a live smoke test.
It is intentionally **not** a member of the root cargo workspace (its own empty
`[workspace]` in `Cargo.toml` detaches it).

## Build / deploy (needs toolchain)

```sh
cargo install worker-build wrangler
# edit wrangler.toml [vars]; set secrets:
wrangler secret put SPKY_DB_AUTH
wrangler secret put SPKY_AUTH_SECRET
wrangler deploy
```

## Open before production

- Confirm SurrealDB Cloud permits `DEFINE EVENT ... http::post(... /ingest)`
  egress to this DO (spike 2: mechanism works, gated by allow-net — Cloud
  managed-policy TBD).
- Live smoke: bootstrap → ingest → view delta → force eviction → warm restart.
- Alarm granularity vs the edge-flush window; DO storage limits vs snapshot size.
