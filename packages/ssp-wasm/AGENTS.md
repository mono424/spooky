# `@spooky-sync/ssp-wasm` — agent guide

## What this package is

The browser-side stream processor: a Rust crate compiled to WebAssembly via `wasm-pack` that runs the same DBSP-style materialized-view circuit as the Rust `apps/ssp` server, but inside the user's tab. `@spooky-sync/core` instantiates it under the hood to power local reactive queries — you almost never touch this package directly.

## When you might touch it

- You're debugging why a local query doesn't update after a mutation. `ingest()` is where new records land.
- You're testing the surrealism (embedded WASM) generation mode of `spky generate` and need to inspect the bundled circuit.
- You're saving/restoring local state across page loads outside the built-in IndexedDB persistence path (`save_state` / `load_state`).

## Public API (`pkg/ssp_wasm.d.ts`)

- **`class Sp00kyProcessor`** (constructed once, stored on the client):
  - `register_view(config: WasmViewConfig)` — register a materialized view by ID + SurQL + params.
  - `unregister_view(id)` — remove a view.
  - `ingest(table, op, id, record)` — push a row change; returns the affected views' deltas (`WasmViewUpdate[]`).
  - `save_state()` / `load_state(json)` — serialize/restore the circuit.
  - `free()` / `[Symbol.dispose]` — release WASM memory.
- **`init()`** — must be called once after the WASM module loads.
- Types: `WasmViewConfig`, `WasmViewUpdate`, `WasmIngestItem`.

## Build

```bash
pnpm --filter @spooky-sync/ssp-wasm build   # wasm-pack build --target web --out-dir pkg
```

Output is `pkg/`, which is the only directory shipped via `files`. The Rust source lives one level up in `packages/ssp/src/circuit/`.

## Pointers

- Server-side counterpart (same circuit, native build): `apps/ssp/`
- Sync engine that wires this in: `node_modules/@spooky-sync/core/AGENTS.md` → `src/modules/data/`
