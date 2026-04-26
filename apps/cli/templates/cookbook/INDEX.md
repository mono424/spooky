# sp00ky cookbook

A short, scannable list of patterns an AI agent (or human) reaches for when writing code against a sp00ky-backed app. Each entry has a one-sentence "when to use," the canonical snippet, and one gotcha.

Render any recipe with:
```
spky scaffold <recipe> --table <your-table>
```
or pass `--out path/to/file.tsx` to write the snippet directly.

## Recipes

### `live-list`
**When to use:** you want a reactive, sorted list of rows from a table that updates as records change.
**Render:** `spky scaffold live-list --table thread`
**Gotcha:** end the query with `.build()` — `useQuery` will hang on a bare builder.

### `optimistic-mutation`
**When to use:** you're inserting a new record from a UI action (form submit, button click) and want the local cache to update immediately while the mutation drains to the remote.
**Render:** `spky scaffold optimistic-mutation --table thread`
**Gotcha:** `db.create` takes a *full* record-ID string (`'thread:abc'`), not `(table, payload)`. Use `Uuid` to mint IDs.

### `crdt-text-field`
**When to use:** a text column is annotated `-- @crdt text` in your schema and you need a collaborative editor wired to it.
**Render:** `spky scaffold crdt-text-field --table thread --field content`
**Gotcha:** never read the field via `useQuery`; always `useCrdtField`. Writes must pass `{ debounced: true }` to `db.update` so rapid keystrokes coalesce.

## Related

- See `AGENTS.md` (in your project root or `node_modules/@spooky-sync/*/AGENTS.md`) for the broader mental model and gotchas.
- After editing the schema: `spky generate` then `spky doctor` to confirm everything is in sync.
