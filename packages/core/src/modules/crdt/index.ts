import type { SchemaStructure } from '@spooky-sync/query-builder';
import type { RemoteDatabaseService } from '../../services/database/index';
import type { Logger } from '../../services/logger/index';
import type { Uuid } from 'surrealdb';
import { CrdtField } from './crdt-field';
import { parseRecordIdString } from '../../utils/index';

export { CrdtField } from './crdt-field';

/**
 * CrdtManager manages active CrdtField instances and their sync channels.
 *
 * Each open record gets a LIVE SELECT on _00_crdt that delivers remote
 * changes in real time.
 */
export class CrdtManager {
  private fields = new Map<string, CrdtField>();
  private liveQueries = new Map<string, { uuid: Uuid; table: string }>();
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

    crdtField = new CrdtField(field, initialCrdtState, fallbackText);
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

    const hasOtherFields = Array.from(this.fields.keys()).some(
      (k) => k !== key && k.includes(recordId)
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
    this.liveQueries.clear();
  }

  private async ensureLiveSelect(table: string, recordId: string): Promise<void> {
    if (this.liveQueries.has(recordId)) return;

    try {
      const [uuid] = await this.remote.query<[Uuid]>(
        `LIVE SELECT * FROM _00_crdt WHERE record_id = ${recordId}`,
      );

      this.liveQueries.set(recordId, { uuid, table });

      const subscription = await this.remote.getClient().liveOf(uuid);
      subscription.subscribe((message) => {
        if (message.action === 'KILLED') return;

        if (message.action === 'CREATE' || message.action === 'UPDATE') {
          const fieldName = message.value.field as string;
          const state = message.value.state as string;

          if (!fieldName || !state) return;

          const key = this.makeKey(table, recordId, fieldName);
          const crdtField = this.fields.get(key);
          if (crdtField) {
            this.logger.debug(
              { recordId, field: fieldName, action: message.action, Category: 'sp00ky-client::CrdtManager::liveSelect' },
              'LIVE SELECT delivered CRDT update'
            );
            crdtField.importRemote(state);
          }
        }
      });

      this.logger.info(
        { recordId, Category: 'sp00ky-client::CrdtManager::ensureLiveSelect' },
        'LIVE SELECT on _00_crdt started'
      );
    } catch (e) {
      this.logger.warn(
        { error: e, recordId, Category: 'sp00ky-client::CrdtManager::ensureLiveSelect' },
        'Failed to start LIVE SELECT on _00_crdt'
      );
    }
  }

  private killLiveSelect(recordId: string): void {
    const entry = this.liveQueries.get(recordId);
    if (entry) {
      this.remote.query('KILL $uuid', { uuid: entry.uuid }).catch(() => {});
      this.liveQueries.delete(recordId);
    }
  }

  private makeKey(table: string, recordId: string, field: string): string {
    return `${table}:${recordId}:${field}`;
  }
}
