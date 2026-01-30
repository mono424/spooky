import { LocalDatabaseService, RemoteDatabaseService } from '../../services/database/index.js';
import { Logger } from '../../services/logger/index.js';
import { SchemaStructure } from '@spooky/query-builder';
import { RecordId } from 'surrealdb';
import { StreamUpdate, StreamUpdateReceiver } from '../../services/stream-processor/index.js';
import { encodeRecordId } from '../../utils/index.js';

// DevTools interfaces (matching extension expectations)
export interface DevToolsEvent {
  id: number;
  timestamp: number;
  eventType: string;
  payload: any;
}

import { DataModule } from '../data/index.js';
import { AuthService } from '../auth/index.js';
import { AuthEventTypes } from '../auth/events/index.js';

export class DevToolsService implements StreamUpdateReceiver {
  private eventsHistory: DevToolsEvent[] = [];
  private eventIdCounter = 0;
  private version = '1.0.0';

  constructor(
    private databaseService: LocalDatabaseService,
    private remoteDatabaseService: RemoteDatabaseService,
    private logger: Logger,
    private schema: SchemaStructure,
    private authService: AuthService<SchemaStructure>,
    private dataManager?: DataModule<SchemaStructure>
  ) {
    this.exposeToWindow();

    // Subscribe to auth events
    this.authService.eventSystem.subscribe(AuthEventTypes.AuthStateChanged, () => {
      this.notifyDevTools();
    });

    this.logger.debug({ Category: 'spooky-client::DevToolsService::init' }, 'Service initialized');
  }

  // Get active queries directly from DataManager (single source of truth)
  private getActiveQueries(): Map<number, any> {
    const result = new Map<number, any>();
    if (!this.dataManager) return result;

    const queries = this.dataManager.getActiveQueries();
    queries.forEach((q) => {
      const queryHash = this.hashString(encodeRecordId(q.config.id));
      result.set(queryHash, {
        queryHash,
        status: 'active',
        createdAt:
          q.config.lastActiveAt instanceof Date
            ? q.config.lastActiveAt.getTime()
            : new Date(q.config.lastActiveAt || Date.now()).getTime(),
        lastUpdate: Date.now(),
        updateCount: q.updateCount,
        query: q.config.surql,
        variables: q.config.params || {},
        dataSize: q.records?.length || 0,
        data: q.records,
        localArray: q.config.localArray,
        remoteArray: q.config.remoteArray,
      });
    });
    return result;
  }

  public onQueryInitialized(payload: any) {
    this.logger.debug(
      { payload, Category: 'spooky-client::DevToolsService::onQueryInitialized' },
      'QueryInitialized'
    );
    const queryHash = this.hashString(payload.queryId.toString());

    this.addEvent('QUERY_REQUEST_INIT', {
      queryHash,
      query: payload.sql,
      variables: {},
    });
    this.notifyDevTools();
  }

  public onQueryUpdated(payload: any) {
    this.logger.debug(
      {
        id: payload.queryId?.toString(),
        Category: 'spooky-client::DevToolsService::onQueryUpdated',
      },
      'QueryUpdated'
    );
    const queryHash = this.hashString(payload.queryId.toString());

    this.addEvent('QUERY_UPDATED', {
      queryHash,
      data: payload.records,
    });
    this.notifyDevTools();
  }

  public onStreamUpdate(update: StreamUpdate) {
    this.logger.debug(
      { update, Category: 'spooky-client::DevToolsService::onStreamUpdate' },
      'StreamUpdate'
    );
    this.addEvent('STREAM_UPDATE', {
      updates: [update],
    });
    this.notifyDevTools();
  }

  public onMutation(payload: any[]) {
    const payloads = payload;
    payloads.forEach((p) => {
      this.addEvent('MUTATION_REQUEST_EXECUTION', {
        mutation: {
          type: 'create', // simplifying
          data: 'data' in p ? p.data : undefined,
          selector: encodeRecordId(p.record_id),
        },
      });
    });
    this.notifyDevTools();
  }

  private hashString(str: string): number {
    let hash = 0;
    if (str.length === 0) return hash;
    for (let i = 0; i < str.length; i++) {
      const char = str.charCodeAt(i);
      hash = (hash << 5) - hash + char;
      hash = hash & hash; // Convert to 32bit integer
    }
    return hash;
  }

  public logEvent(eventType: string, payload: any) {
    this.addEvent(eventType, payload);
    this.notifyDevTools();
  }

  private addEvent(eventType: string, payload: any) {
    this.eventsHistory.push({
      id: this.eventIdCounter++,
      timestamp: Date.now(),
      eventType,
      payload: this.serializeForDevTools(payload),
    });
    if (this.eventsHistory.length > 100) this.eventsHistory.shift();
  }

  private getState() {
    return this.serializeForDevTools({
      eventsHistory: [...this.eventsHistory],
      activeQueries: Object.fromEntries(this.getActiveQueries()),
      auth: {
        authenticated: this.authService.isAuthenticated,
        userId: this.authService.currentUser?.id,
      },
      version: this.version,
      database: {
        tables: this.schema.tables.map((t) => t.name),
        tableData: {},
      },
    });
  }

  private notifyDevTools() {
    if (typeof window !== 'undefined') {
      window.postMessage(
        {
          type: 'SPOOKY_STATE_CHANGED',
          source: 'spooky-devtools-page',
          state: this.getState(),
        },
        '*'
      );
    }
  }

  private serializeForDevTools(data: any, seen = new WeakSet<object>()): any {
    if (data === undefined) {
      return 'undefined';
    }

    if (data === null) {
      return null;
    }

    if (data instanceof RecordId) {
      return data.toString();
    }

    if (Array.isArray(data)) {
      if (seen.has(data)) {
        return '[Circular Array]';
      }
      seen.add(data);
      return data.map((item) => this.serializeForDevTools(item, seen));
    }

    if (typeof data === 'bigint') {
      return data.toString();
    }

    if (data instanceof Date) {
      return data.toISOString();
    }

    if (typeof data === 'object') {
      if (seen.has(data)) {
        return '[Circular Object]';
      }
      seen.add(data);

      const result: Record<string, any> = {};
      for (const key in data) {
        if (Object.prototype.hasOwnProperty.call(data, key)) {
          result[key] = this.serializeForDevTools(data[key], seen);
        }
      }
      return result;
    }

    return data;
  }

  private exposeToWindow() {
    if (typeof window !== 'undefined') {
      (window as any).__SPOOKY__ = {
        version: this.version,
        getState: () => this.getState(),
        clearHistory: () => {
          this.eventsHistory = [];
          this.notifyDevTools();
        },
        getTableData: async (tableName: string) => {
          try {
            // Returns the first statement result as T.
            // SurrealDB query returns [Result1, Result2...].
            // We want the records from the first result.
            const result = await this.databaseService.query<any>(`SELECT * FROM ${tableName}`);

            let records: any[] = [];

            if (Array.isArray(result) && result.length > 0) {
              const first = result[0];
              if (Array.isArray(first)) {
                // Legacy or flattened format: [[records]]
                records = first;
              } else if (
                first &&
                typeof first === 'object' &&
                'result' in first &&
                'status' in first
              ) {
                // SurrealDB 2.0 format: [{ result: [...records], status: 'OK', ... }]
                records = Array.isArray(first.result) ? first.result : [];
              } else {
                // Fallback: assume result is the array of records itself
                records = result;
              }
            } else if (Array.isArray(result)) {
              // Empty array
              records = [];
            }

            return this.serializeForDevTools(records) || [];
          } catch (e) {
            this.logger.error(
              { err: e, Category: 'spooky-client::DevToolsService::exposeToWindow' },
              'Failed to get table data'
            );
            return [];
          }
        },
        updateTableRow: async (
          tableName: string,
          recordId: string,
          updates: Record<string, unknown>
        ) => {
          try {
            await this.databaseService.query(`UPDATE ${recordId} MERGE $updates`, { updates });
            return { success: true };
          } catch (e: any) {
            return { success: false, error: e.message };
          }
        },
        deleteTableRow: async (tableName: string, recordId: string) => {
          try {
            await this.databaseService.query(`DELETE ${recordId}`);
            return { success: true };
          } catch (e: any) {
            return { success: false, error: e.message };
          }
        },
        runQuery: async (query: string, target: 'local' | 'remote' = 'local') => {
          try {
            this.logger.debug(
              { query, target, Category: 'spooky-client::DevToolsService::runQuery' },
              'Running query (START)'
            );
            const service = target === 'remote' ? this.remoteDatabaseService : this.databaseService;

            const startTime = Date.now();
            const result = await service.query<any>(query);
            const queryTime = Date.now() - startTime;

            this.logger.debug(
              {
                query,
                time: queryTime,
                resultType: typeof result,
                isArray: Array.isArray(result),
                Category: 'spooky-client::DevToolsService::runQuery',
              },
              'Database returned result'
            );

            // Serialize the result for DevTools
            const serializeStart = Date.now();
            const serialized = this.serializeForDevTools(result);
            const serializeTime = Date.now() - serializeStart;

            this.logger.debug(
              {
                serializeTime,
                serializedLength: JSON.stringify(serialized).length,
                Category: 'spooky-client::DevToolsService::runQuery',
              },
              'Serialization complete'
            );

            return {
              success: true,
              data: serialized,
              target,
            };
          } catch (e: any) {
            this.logger.error(
              { err: e, query, target, Category: 'spooky-client::DevToolsService::runQuery' },
              'Query execution failed'
            );
            // Ensure we always return a string for error
            const errorMessage =
              e instanceof Error ? e.message : typeof e === 'string' ? e : JSON.stringify(e);
            return { success: false, error: errorMessage || 'Unknown occurred' };
          }
        },
      };

      window.postMessage(
        {
          type: 'SPOOKY_DETECTED',
          source: 'spooky-devtools-page',
          data: { version: this.version, detected: true },
        },
        '*'
      );
    }
  }
}
