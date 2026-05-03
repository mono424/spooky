import type { SchemaStructure } from '@spooky-sync/query-builder';
import type { RemoteDatabaseService } from '../../services/database/index';
import type { Logger } from '../../services/logger/index';
import type { Uuid } from 'surrealdb';
import { CrdtField } from './crdt-field';
import { parseRecordIdString } from '../../utils/index';

export { CrdtField, cursorColorFromName, CURSOR_COLORS } from './crdt-field';

/**
 * CrdtManager manages active CrdtField instances and their sync channels.
 *
 * Each open record gets two LIVE SELECTs:
 *  - _00_crdt: persistent CRDT field snapshots (visible to anyone with parent
 *    SELECT permission)
 *  - _00_cursor: ephemeral cursor / presence state (only visible to callers
 *    with parent UPDATE permission)
 */
export class CrdtManager {
  private fields = new Map<string, CrdtField>();
  // Per recordId: one entry per source table we're subscribed to.
  private liveQueries = new Map<string, { uuid: Uuid; table: string }[]>();
  private logger: Logger;

  constructor(
    private schema: SchemaStructure,
    private remote: RemoteDatabaseService,
    logger: Logger
  ) {
    this.logger = logger.child({ service: 'CrdtManager' });
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

    // Load saved CRDT state from remote _00_crdt table
    let initialCrdtState: string | undefined;
    try {
      const [result] = await this.remote.query<[string[]]>(
        'SELECT VALUE state FROM _00_crdt WHERE record_id = $rid AND field = $field LIMIT 1',
        { rid: parseRecordIdString(recordId), field }
      );
      if (result && result.length > 0 && result[0]) {
        initialCrdtState = result[0];
      }
    } catch (e) {
      this.logger.debug(
        { error: e, Category: 'sp00ky-client::CrdtManager::open' },
        'No existing CRDT state found'
      );
    }

    crdtField = new CrdtField(field, initialCrdtState, this.logger);
    crdtField.startSync(this.remote, recordId);
    this.fields.set(key, crdtField);

    this.logger.info(
      { key, hasInitialState: !!initialCrdtState, hasFallback: !!fallbackText, Category: 'sp00ky-client::CrdtManager::open' },
      'CrdtField opened'
    );

    await this.ensureLiveSelect(table, recordId);

    return crdtField;
  }

  close(table: string, recordId: string, field: string): void {
    const key = this.makeKey(table, recordId, field);
    const crdtField = this.fields.get(key);
    if (crdtField) {
      crdtField.stopSync();
      this.fields.delete(key);
    }

    const prefix = `${table}:${recordId}:`;
    const hasOtherFields = Array.from(this.fields.keys()).some(
      (k) => k !== key && k.startsWith(prefix)
    );
    if (!hasOtherFields) {
      this.killLiveSelect(recordId);
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
    for (const recordId of this.liveQueries.keys()) {
      this.killLiveSelect(recordId);
    }
  }

  private async ensureLiveSelect(table: string, recordId: string): Promise<void> {
    if (this.liveQueries.has(recordId)) return;

    const subscriptions: { uuid: Uuid; table: string }[] = [];

    // Persistent CRDT snapshots — visible to anyone with parent SELECT.
    const crdtUuid = await this.subscribeTo('_00_crdt', recordId, (fieldName, state) => {
      const key = this.makeKey(table, recordId, fieldName);
      this.fields.get(key)?.importRemote(state);
    });
    if (crdtUuid) subscriptions.push({ uuid: crdtUuid, table: '_00_crdt' });

    // Ephemeral cursor state — table-level permission limits this to callers
    // with parent UPDATE, so a read-only viewer never even gets the LIVE feed.
    const cursorUuid = await this.subscribeTo('_00_cursor', recordId, (fieldName, state) => {
      const key = this.makeKey(table, recordId, fieldName);
      this.fields.get(key)?.importRemoteCursor(state);
    });
    if (cursorUuid) subscriptions.push({ uuid: cursorUuid, table: '_00_cursor' });

    if (subscriptions.length > 0) {
      this.liveQueries.set(recordId, subscriptions);
    }
  }

  /** Start one LIVE SELECT for `recordId` on `tableName` and dispatch each
   *  CREATE/UPDATE row to `onRow(field, state)`. Returns the live UUID, or
   *  `undefined` if the subscription couldn't be started (e.g. permission
   *  denied — expected for cursors when the caller is read-only). */
  private async subscribeTo(
    tableName: '_00_crdt' | '_00_cursor',
    recordId: string,
    onRow: (field: string, state: string) => void,
  ): Promise<Uuid | undefined> {
    try {
      const [uuid] = await this.remote.query<[Uuid]>(
        `LIVE SELECT * FROM ${tableName} WHERE record_id = $rid`,
        { rid: parseRecordIdString(recordId) },
      );

      const subscription = await this.remote.getClient().liveOf(uuid);
      subscription.subscribe((message) => {
        if (message.action === 'KILLED') return;
        if (message.action !== 'CREATE' && message.action !== 'UPDATE') return;
        const fieldName = message.value.field as string;
        const state = message.value.state as string;
        if (!fieldName || !state) return;
        onRow(fieldName, state);
      });

      this.logger.info(
        { recordId, table: tableName, Category: 'sp00ky-client::CrdtManager::ensureLiveSelect' },
        'LIVE SELECT started'
      );
      return uuid;
    } catch (e) {
      this.logger.warn(
        { error: e, recordId, table: tableName, Category: 'sp00ky-client::CrdtManager::ensureLiveSelect' },
        'Failed to start LIVE SELECT'
      );
      return undefined;
    }
  }

  private killLiveSelect(recordId: string): void {
    const entries = this.liveQueries.get(recordId);
    if (entries) {
      for (const entry of entries) {
        this.remote.query('KILL $uuid', { uuid: entry.uuid }).catch(() => {});
      }
      this.liveQueries.delete(recordId);
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
