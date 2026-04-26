# `@spooky-sync/cli` (`spky`) — agent guide

## What this package is

The sp00ky toolchain. A Rust binary (`spky`) plus a thin npm wrapper. It parses `.surql` schemas, emits typed `schema.gen.ts` (and `.dart`), runs migrations, manages buckets and API backends, drives the local dev environment, and orchestrates Sp00ky Cloud deployments.

## Binary

```
spky <subcommand> [flags]
```

Installed via `npx @spooky-sync/cli` or globally as `spky`. The `bin` field in `package.json` is `spky` — *not* `sp00ky`.

## Project layout it expects

```
your-app/
├── sp00ky.yml              # config: schema path, generated outputs, backends, buckets
├── schema/
│   └── schema.surql        # source of truth — your domain model
├── src/
│   └── schema.gen.ts       # GENERATED — never hand-edit
└── migrations/             # GENERATED migrations; modified files are tracked by checksum
```

`spky` finds `sp00ky.yml` in the current directory by default; pass `--config <path>` to override.

## Subcommands an app developer/agent uses most

- **`spky generate` / `spky gen`** — read `sp00ky.yml`, parse all `.surql`, emit `schema.gen.ts` (and Dart equivalents per config). **Run this after every schema edit.**
- **`spky migrate create <name>`** — diff current schema against the last applied migration and emit a new `.surql` migration file.
- **`spky migrate apply`** — apply pending migrations against the configured database. `--fix-checksums` updates stored checksums for legitimately-modified migration files.
- **`spky migrate status`** — show pending vs applied vs modified-but-applied migrations.
- **`spky migrate fix [--fix-checksums]`** — repair schema drift / checksum mismatches.
- **`spky verify [--fix]`** — confirm SSP/scheduler snapshot matches upstream SurrealDB. `--fix` triggers a resync.
- **`spky lint`** — validate `sp00ky.yml` and referenced files exist.
- **`spky dev [--apply-migrations] [--clean]`** — boots a local SurrealDB + SSP + scheduler stack via Docker. `--clean` wipes SSP/scheduler state but preserves user data in SurrealDB.
- **`spky create`** — scaffold a new sp00ky project.
- **`spky bucket add`** / **`spky api add`** — append a bucket or backend definition to `sp00ky.yml`.
- **`spky mcp`** — start the bundled `@spooky-sync/devtools-mcp` server (so AI assistants can introspect the running app).

## Cloud subcommands (deployment)

`spky cloud login | create | deploy | status | logs | scale | restart | destroy | backup | env | keys | link | team | vault | credentials`. See `spky cloud --help`. Most app code agents touch never need these.

## Schema annotations the parser recognizes

In your `.surql` source, comment annotations attached to `DEFINE FIELD` / `DEFINE TABLE` change codegen output:

- `-- @crdt text` (above a `DEFINE FIELD`) — marks a field as a Loro CRDT text field. Consumers must use `useCrdtField` to read/write it; plain `useQuery` will see stale or unmerged content.
- `-- @parent` (suffix on `DEFINE FIELD ... TYPE record<...>`) — marks the column as the parent side of a relationship; written automatically from the auth context, never by client code.

Example:
```sql
DEFINE TABLE thread SCHEMAFULL ...;

-- @crdt text
DEFINE FIELD content ON TABLE thread TYPE string ASSERT $value != NONE;

DEFINE FIELD author ON TABLE thread TYPE record<user>; -- @parent
```

## Common gotchas

- **`schema.gen.ts` must be regenerated after every `.surql` change.** `spky generate`. CI typically asserts no drift.
- **Migrations are checksum-tracked.** Editing a previously-applied migration file won't silently re-run; `spky migrate status` flags it. Use `--fix-checksums` only when you're sure the change is semantically a no-op.
- **`sp00ky.yml` is the entry point.** The CLI never crawls for `.surql`; everything is wired explicitly through the config.
- **Generation modes matter.** The `--mode` flag (`singlenode`, `cluster`, `surrealism`) changes what the generated client connects to. Default is `singlenode` (HTTP to a single SSP). `surrealism` embeds the WASM stream processor in-browser.
- **Don't commit the bin output.** The Rust binary is built per-platform and shipped via the npm tarball under `dist/`.

## Pointers

- Sync engine the generated client targets: `node_modules/@spooky-sync/core/AGENTS.md`
- Reactive UI bindings: `node_modules/@spooky-sync/client-solid/AGENTS.md`
- Live MCP introspection during dev: `node_modules/@spooky-sync/devtools-mcp/AGENTS.md`
