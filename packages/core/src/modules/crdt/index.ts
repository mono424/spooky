import type { SchemaStructure } from '@spooky-sync/query-builder';
import type { LocalDatabaseService, RemoteDatabaseService } from '../../services/database/index';
import type { Logger } from '../../services/logger/index';
import type { Uuid } from 'surrealdb';
import { CrdtField } from './crdt-field';
import { parseRecordIdString } from '../../utils/index';

export { CrdtField, cursorColorFromName, CURSOR_COLORS } from './crdt-field';

/**
 * CrdtManager manages active CrdtField instances and their sync channels.
 *
 * Collaborative state lives in two dedicated tables (defined in
 * `apps/cli/src/meta_tables_remote.surql`):
 *   - `_00_crdt`   { record_id, field, state } — one row per (record, field)
 *   - `_00_cursor` { record_id, session_id, field, state } — one row per
 *                    (record, session, field)
 *
 * Splitting them off the parent row is what makes offline edits mergeable:
 * each (record, field) gets its own row, so concurrent offline writes don't
 * collide on the parent's last-write-wins semantics.
 *
 * Cross-browser delivery still rides the parent table's existing LIVE feed
 * to avoid SurrealDB v3 LIVE bugs around dereference-based permission rules
 * (issues 3602, 4026). On every meta UPSERT the writer also bumps the
 * parent's `_00_rv` (a no-op assignment); that fires the parent's LIVE
 * feed, and the receiver pulls the matching `_00_crdt` / `_00_cursor` rows
 * via subquery. Permission inheritance happens server-side via
 * `record_id.id != NONE` (SELECT) and `fn::can_update_record` (UPDATE).
 */
export class CrdtManager {
  private fields = new Map<string, CrdtField>();
  // One LIVE subscription per parent table (e.g. "thread" → uuid).
  private liveByTable = new Map<string, Uuid>();
  // Coalesces concurrent first-time subscribes for the same table.
  private pendingLive = new Map<string, Promise<void>>();
  private logger: Logger;
  // SurrealDB session id, used as the per-session key inside `_00_cursor`.
  private sessionId: string = '';

  constructor(
    private schema: SchemaStructure,
    private local: LocalDatabaseService,
    private remote: RemoteDatabaseService,
    logger: Logger,
    private debounceMs: number = 500,
  ) {
    this.logger = logger.child({ service: 'CrdtManager' });
  }

  /** Set the session id that scopes this client's cursor entries. Must be
   *  called before `open()` for cursors to be pushed under a stable key.
   *  Passed in from `sp00ky.ts` at boot (it already fetches `session::id()`
   *  for the data-module salt). */
  setSessionId(sessionId: string): void {
    this.sessionId = sessionId;
  }

  /**
   * Open a CRDT field for collaborative editing.
   *
   * @param table - Table name
   * @param recordId - Full record ID (e.g., "thread:abc")
   * @param field - Field name (e.g., "title", "content")
   * @param fallbackText - Current plain text from the record, used to seed the
   *                       LoroDoc if no CRDT state exists yet (migration path)
   */
  async open(
    table: string,
    recordId: string,
    field: string,
    fallbackText?: string,
  ): Promise<CrdtField> {
    this.assertCrdtField(table, field);
    const cursorsEnabled = this.fieldHasCursor(table, field);
    const key = this.makeKey(table, recordId, field);
    let crdtField = this.fields.get(key);

    if (crdtField) {
      return crdtField;
    }

    // Read the snapshot directly off the parent row. `@crdt`-only fields
    // hold the base64 snapshot inline; `@crdt @cursor` fields hold a
    // `{ state, cursors }` object so we drill into `.state`. The query
    // always selects the local row — sync-down already populated it
    // (the snapshot is a column on the parent, not a sidecar table) so
    // there is no separate fetch on the happy path.
    let initialCrdtState: Uint8Array | undefined;
    try {
      const [result] = await this.local.query<[unknown]>(
        `SELECT VALUE ${field} FROM ONLY $id`,
        { id: parseRecordIdString(recordId) },
      );
      const snapshot = this.extractSnapshot(result, cursorsEnabled);
      if (snapshot) initialCrdtState = snapshot;
    } catch (e) {
      this.logger.info(
        { error: String(e), recordId, field, Category: 'sp00ky-client::CrdtManager::open' },
        'No existing CRDT state found in local cache (continuing with empty doc)'
      );
    }

    crdtField = new CrdtField(field, cursorsEnabled, initialCrdtState, this.logger);
    crdtField.startSync(this.local, this.remote, recordId, this.sessionId, this.debounceMs);
    this.fields.set(key, crdtField);

    this.logger.info(
      { key, hasInitialState: !!initialCrdtState, hasFallback: !!fallbackText, Category: 'sp00ky-client::CrdtManager::open' },
      'CrdtField opened'
    );

    // Fire-and-forget: the LIVE subscription receives *future* updates;
    // the initial snapshot is already in hand. `ensureTableSubscription`
    // coalesces concurrent calls via `pendingLive`, so this is safe.
    void this.ensureTableSubscription(table);

    // Local was empty — a fresh device, a memory-backed local DB after
    // reload, or a record that hasn't been sync'd locally yet. Pull the
    // parent row from remote and dispatch its CRDT field; otherwise the
    // editor sits empty until the parent's LIVE feed happens to fire.
    if (!initialCrdtState) {
      void this.fetchAndDispatchRow(table, recordId);
    }

    return crdtField;
  }

  close(table: string, recordId: string, field: string): void {
    const key = this.makeKey(table, recordId, field);
    const crdtField = this.fields.get(key);
    if (crdtField) {
      crdtField.stopSync();
      this.fields.delete(key);
    }

    // If no fields on this table remain open, tear down the table-wide LIVE.
    const tablePrefix = `${table}:`;
    const stillOpen = Array.from(this.fields.keys()).some((k) => k.startsWith(tablePrefix));
    if (!stillOpen) {
      this.killTableSubscription(table);
    }

    this.logger.debug(
      { key, Category: 'sp00ky-client::CrdtManager::close' },
      'CrdtField closed'
    );
  }

  closeAll(): void {
    for (const [_, field] of this.fields) {
      field.stopSync();
    }
    this.fields.clear();
    for (const table of Array.from(this.liveByTable.keys())) {
      this.killTableSubscription(table);
    }
  }

  /** Ensure a single `LIVE SELECT * FROM <table>` is running, shared across
   *  every open CrdtField on `table`. */
  private async ensureTableSubscription(table: string): Promise<void> {
    if (this.liveByTable.has(table)) return;

    const pending = this.pendingLive.get(table);
    if (pending) return pending;

    const start = (async () => {
      try {
        const [uuid] = await this.remote.query<[Uuid]>(
          `LIVE SELECT * FROM ${table}`,
        );

        const subscription = await this.remote.getClient().liveOf(uuid);
        subscription.subscribe((message) => {
          if (message.action === 'KILLED') return;
          if (message.action !== 'CREATE' && message.action !== 'UPDATE') return;
          this.dispatchRow(table, message.value as Record<string, unknown>);
        });

        this.liveByTable.set(table, uuid);
        this.logger.info(
          { table, Category: 'sp00ky-client::CrdtManager::ensureTableSubscription' },
          'LIVE SELECT started'
        );
      } catch (e) {
        this.logger.warn(
          { error: e, table, Category: 'sp00ky-client::CrdtManager::ensureTableSubscription' },
          'Failed to start LIVE SELECT'
        );
      }
    })();

    this.pendingLive.set(table, start);
    try {
      await start;
    } finally {
      this.pendingLive.delete(table);
    }
  }

  /** Apply a parent-row payload from a non-LIVE source (e.g. the
   *  list_ref-driven sync engine, when the cross-user LIVE on the
   *  parent table is filtered out by the SurrealDB cross-session
   *  permission gap). Same semantics as the internal `dispatchRow`. */
  applyRow(table: string, row: Record<string, unknown>): void {
    this.dispatchRow(table, row);
  }

  /** Dispatch a parent-row LIVE event to every open CrdtField on that
   *  record. Each open field reads its slice of the row directly — the
   *  CRDT snapshot is a column on the parent now, so there is no
   *  follow-up subquery. */
  private dispatchRow(table: string, row: Record<string, unknown>): void {
    const id = row.id != null ? String(row.id) : '';
    if (!id) return;

    const rowKeyPrefix = `${table}:${id}:`;
    for (const [key, crdtField] of this.fields) {
      if (!key.startsWith(rowKeyPrefix)) continue;
      const fieldName = key.slice(rowKeyPrefix.length);
      const cursorsEnabled = this.fieldHasCursor(table, fieldName);
      const slice = row[fieldName];
      const snapshot = this.extractSnapshot(slice, cursorsEnabled);
      if (snapshot) crdtField.importRemote(snapshot);

      if (cursorsEnabled && slice && typeof slice === 'object') {
        const cursors = (slice as { cursors?: unknown }).cursors;
        if (cursors && typeof cursors === 'object') {
          for (const [sid, blob] of Object.entries(cursors as Record<string, unknown>)) {
            if (sid === this.sessionId) continue;
            if (typeof blob === 'string' && blob.length > 0) {
              crdtField.importRemoteCursor(blob);
            }
          }
        }
      }
    }
  }

  /** One-shot remote fetch for a row whose CRDT field hasn't synced
   *  locally yet (fresh device, memory-backed local DB after reload, …).
   *  Used by `open()` when the local read came up empty. Subsequent
   *  cross-browser updates ride `dispatchRow` via the parent LIVE feed. */
  private async fetchAndDispatchRow(table: string, id: string): Promise<void> {
    try {
      const recordId = parseRecordIdString(id);
      const [row] = await this.remote.query<[Record<string, unknown> | null]>(
        `SELECT * FROM ONLY $id`,
        { id: recordId },
      );
      if (!row || typeof row !== 'object') return;
      this.dispatchRow(table, row as Record<string, unknown>);
    } catch (e) {
      this.logger.warn(
        { error: e, table, id, Category: 'sp00ky-client::CrdtManager::fetchAndDispatchRow' },
        'Failed to fetch parent row for CRDT hydration'
      );
    }
  }

  /** Schema lookup: does `<table>.<field>` carry a `@cursor` annotation?
   *  Determines the on-disk shape (plain snapshot vs. `{ state, cursors }`). */
  private fieldHasCursor(table: string, field: string): boolean {
    const tableSchema = this.schema.tables.find((t) => t.name === table);
    return !!tableSchema?.columns[field]?.cursor;
  }

  /** Pull the LoroDoc snapshot bytes out of a row slice. For `@crdt`-only
   *  the slice IS the snapshot (Uint8Array); for `@crdt @cursor` it's
   *  `{ state, cursors }` where `state` carries the snapshot bytes. */
  private extractSnapshot(value: unknown, cursorsEnabled: boolean): Uint8Array | undefined {
    const asBytes = (v: unknown): Uint8Array | undefined => {
      if (v instanceof Uint8Array) return v.length > 0 ? v : undefined;
      // SurrealDB ferries bytes through several shapes depending on
      // transport and on whether the field is a top-level `bytes` column
      // or bytes nested inside a FLEXIBLE object. Round-tripping bytes
      // through `option<object> FLEXIBLE` (the `@crdt @cursor` shape) in
      // particular comes back as a plain `number[]` from the local WASM
      // DB and as `Uint8Array` from the remote WS engine. Normalize all
      // recognized variants here so the receiving CrdtField doesn't care.
      if (v instanceof ArrayBuffer) return new Uint8Array(v);
      if (ArrayBuffer.isView(v)) {
        const view = v as ArrayBufferView;
        return new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
      }
      if (Array.isArray(v) && v.length > 0 && v.every((n) => typeof n === 'number')) {
        return Uint8Array.from(v as number[]);
      }
      return undefined;
    };

    if (cursorsEnabled) {
      if (value && typeof value === 'object' && !(value instanceof Uint8Array)) {
        return asBytes((value as { state?: unknown }).state);
      }
      return undefined;
    }
    return asBytes(value);
  }

  private killTableSubscription(table: string): void {
    const uuid = this.liveByTable.get(table);
    if (uuid) {
      this.remote.query('KILL $uuid', { uuid }).catch(() => {});
      this.liveByTable.delete(table);
    }
  }

  private makeKey(table: string, recordId: string, field: string): string {
    return `${table}:${recordId}:${field}`;
  }

  /**
   * Throws if `<table>.<field>` is not annotated `@crdt` in the schema. Catches
   * typos, removed annotations, and stale schema codegen at the call site instead
   * of silently producing a non-CRDT writer.
   */
  private assertCrdtField(table: string, field: string): void {
    if (!/^[a-zA-Z_][a-zA-Z0-9_]*$/.test(field)) {
      throw new Error(
        `openCrdtField: refusing unsafe field identifier '${field}' — must match [a-zA-Z_][a-zA-Z0-9_]*`
      );
    }
    const tableSchema = this.schema.tables.find((t) => t.name === table);
    if (!tableSchema) {
      throw new Error(
        `openCrdtField: unknown table '${table}'. Available: ${this.schema.tables.map((t) => t.name).join(', ')}`
      );
    }
    const column = tableSchema.columns[field];
    if (!column) {
      throw new Error(
        `openCrdtField: '${table}.${field}' is not in the schema. Available fields: ${Object.keys(tableSchema.columns).join(', ')}`
      );
    }
    if (!column.crdt) {
      throw new Error(
        `openCrdtField: '${table}.${field}' is not annotated '@crdt' in the schema. ` +
          `Add '-- @crdt text' above the field's DEFINE FIELD and regenerate the client schema.`
      );
    }
  }
}
