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
    const key = this.makeKey(table, recordId, field);
    let crdtField = this.fields.get(key);

    if (crdtField) {
      return crdtField;
    }

    // Load the existing CRDT snapshot from the LOCAL cache. In local the
    // state lives as an object field on the parent row itself (injected
    // by apps/cli/src/main.rs alongside `_00_rv`), not as a separate
    // table — the local DB is single-tenant so we don't need the
    // cross-table permission gate the remote design uses. This lets the
    // editor mount offline because the snapshot rides the parent's
    // normal local sync.
    let initialCrdtState: string | undefined;
    try {
      const [result] = await this.local.query<[string | null]>(
        'SELECT VALUE _00_crdt[$field] FROM ONLY $id',
        { id: parseRecordIdString(recordId), field },
      );
      if (typeof result === 'string' && result.length > 0) {
        initialCrdtState = result;
      }
    } catch (e) {
      this.logger.info(
        { error: String(e), recordId, field, Category: 'sp00ky-client::CrdtManager::open' },
        'No existing CRDT state found in local cache (continuing with empty doc)'
      );
    }

    crdtField = new CrdtField(field, initialCrdtState, this.logger);
    crdtField.startSync(this.local, this.remote, recordId, this.sessionId, this.debounceMs);
    this.fields.set(key, crdtField);

    this.logger.info(
      { key, hasInitialState: !!initialCrdtState, hasFallback: !!fallbackText, Category: 'sp00ky-client::CrdtManager::open' },
      'CrdtField opened'
    );

    // Fire-and-forget: the LIVE subscription receives *future* updates;
    // the initial snapshot is already in hand. Awaiting this used to add
    // a network round-trip to every thread open. `ensureTableSubscription`
    // coalesces concurrent calls via `pendingLive`, so this is safe.
    void this.ensureTableSubscription(table);

    // No local snapshot — could be a first-ever open on this device, a
    // memory-backed local DB after reload, or a record that hasn't been
    // touched since `_00_crdt` started being persisted. Pull the remote
    // snapshot now; otherwise the editor sits empty until the parent's
    // LIVE feed happens to fire on a future edit.
    if (!initialCrdtState) {
      void this.fetchAndDispatchMeta(table, recordId);
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

  /** Route a freshly-arrived row to the currently-open CrdtFields. The
   *  parent LIVE feed only carries the parent row itself (the meta tables
   *  are separate), so we follow up with a single round-trip subquery to
   *  pull the matching `_00_crdt` and `_00_cursor` rows and dispatch each
   *  one. */
  private dispatchRow(table: string, row: Record<string, unknown>): void {
    const id = row.id != null ? String(row.id) : '';
    if (!id) return;

    // Skip the round-trip when we have no open editors for this row — there
    // is nothing to deliver to anyway. Keys are `${table}:${recordId}:${field}`,
    // and `id` already includes the table prefix (e.g. "thread:abc").
    const rowKeyPrefix = `${table}:${id}:`;
    const anyOpen = Array.from(this.fields.keys()).some((k) => k.startsWith(rowKeyPrefix));
    if (!anyOpen) return;

    void this.fetchAndDispatchMeta(table, id);
  }

  private async fetchAndDispatchMeta(table: string, id: string): Promise<void> {
    try {
      const recordId = parseRecordIdString(id);
      const [crdtRows, cursorRows] = await this.remote.query<[
        Array<{ field: string; state: string | null }>,
        Array<{ session_id: string; field: string; state: string | null }>,
      ]>(
        `SELECT field, state FROM _00_crdt WHERE record_id = $id;
         SELECT session_id, field, state FROM _00_cursor WHERE record_id = $id;`,
        { id: recordId },
      );

      if (Array.isArray(crdtRows)) {
        for (const r of crdtRows) {
          if (!r || typeof r.state !== 'string' || r.state.length === 0) continue;
          const key = this.makeKey(table, id, r.field);
          this.fields.get(key)?.importRemote(r.state);
        }
      }

      if (Array.isArray(cursorRows)) {
        for (const r of cursorRows) {
          if (!r || typeof r.state !== 'string' || r.state.length === 0) continue;
          if (r.session_id === this.sessionId) continue;
          const key = this.makeKey(table, id, r.field);
          this.fields.get(key)?.importRemoteCursor(r.state);
        }
      }
    } catch (e) {
      this.logger.warn(
        { error: e, table, id, Category: 'sp00ky-client::CrdtManager::fetchAndDispatchMeta' },
        'Failed to fetch CRDT/cursor meta rows after parent LIVE event'
      );
    }
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
