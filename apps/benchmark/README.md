# @spooky-sync/benchmark

Performance benchmark CLI for the sp00ky scheduler + SSP. Measures how the
system scales on two independent axes:

1. **`scale-by-queries`** — fix DB size, sweep registered query count.
2. **`scale-by-rows`** — fix registered queries, sweep DB row count.

The hypothesis being tested: scheduler+SSP load is dominated by registered
query count, not database size, which would mean sp00ky scales horizontally by
sharding queries across SSPs while DB size grows independently.

Each run produces:

- `samples.json` — full raw sample arrays per step
- `summary.md` — Markdown tables of p50/p95/p99 latency, throughput, RSS, and
  scheduler `/metrics` snapshots
- `run.json` — env, git sha, config

## Prerequisites

Run once per repo to make sure the SurrealDB modules + generated schema are
present (the benchmark mirrors the test setup at `tests/surrealdb/setup.ts`):

```sh
pnpm -F @spooky-sync/tests run generate
```

You'll also need Docker running (the SurrealDB stack is brought up with
testcontainers) and the scheduler/SSP binaries — either pre-built (preferred,
much faster startup) or available via `cargo run --release`:

```sh
cargo build --release -p scheduler -p ssp-server
```

If you don't pre-build, the CLI will fall back to `cargo run --release` per
process per step.

## Usage

```sh
pnpm install
pnpm -F @spooky-sync/benchmark build

# scale on registered queries
pnpm -F @spooky-sync/benchmark exec spooky-bench scale-by-queries \
  --queries 10,100,1000 --rows 10000 --ingests-per-step 1000

# scale on DB rows
pnpm -F @spooky-sync/benchmark exec spooky-bench scale-by-rows \
  --queries 500 --rows 1000,10000,100000 --ingests-per-step 1000
```

Useful flags:

- `--scheduler-bin <path>` / `--ssp-bin <path>` — point at pre-built binaries
- `--rss-interval 250ms` / `--metrics-interval 1s` — sampler cadence
- `--verbose` — stream child process stdout/stderr
- `--out <dir>` — output directory (default `./results/<suite>-<timestamp>/`)

## Scope

- Single SSP. Multi-SSP / scheduler load-balancing benchmarks are out of scope
  for this iteration.
- Direct HTTP driver. The TS client SDK is not used (its WASM SurrealDB engine
  is not Node-compatible today).
