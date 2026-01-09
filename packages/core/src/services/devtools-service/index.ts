import { LocalDatabaseService } from '../database/index.js';
import { Logger } from '../logger/index.js';
import { MutationEventSystem, MutationEventTypes } from '../mutation/events.js';
import { QueryEventSystem, QueryEventTypes } from '../query/events.js';
import { SchemaStructure } from '@spooky/query-builder';
import { RecordId } from 'surrealdb';

// DevTools interfaces (matching extension expectations)
export interface DevToolsEvent {
  id: number;
  timestamp: number;
  eventType: string;
  payload: any;
}

import { QueryManager } from '../query/query.js';

export class DevToolsService {
  private eventsHistory: DevToolsEvent[] = [];
  private eventIdCounter = 0;
  private version = '1.0.0'; // TODO: Get from package.json
  private activeQueries = new Map<number, any>();

  constructor(
    // private mutationEvents: MutationEventSystem, // REMOVED
    // private queryEvents: QueryEventSystem, // REMOVED
    private databaseService: LocalDatabaseService,
    private logger: Logger,
    private schema: SchemaStructure,
    private queryManager?: QueryManager<any>
  ) {
    // this.setupEventSubscriptions(); // REMOVED
    this.exposeToWindow();
    this.syncInitialState();
    this.logger.debug('[DevTools] Service initialized');
  }

  private syncInitialState() {
    if (this.queryManager) {
      const queries = this.queryManager.getActiveQueries();
      queries.forEach((q) => {
        const queryHash = this.hashString(q.id.toString());
        this.activeQueries.set(queryHash, {
          queryHash,
          status: 'active',
          createdAt:
            q.lastActiveAt instanceof Date
              ? q.lastActiveAt.getTime()
              : new Date(q.lastActiveAt || Date.now()).getTime(),
          lastUpdate: Date.now(),
          updateCount: 0,
          query: q.surrealql,
          variables: q.params || {},
          dataSize: q.records?.length || 0,
        });
      });
    }
  }

  public onQueryInitialized(payload: any) {
    console.log('[DevTools] IncantationInitialized', payload);
    const queryHash = this.hashString(payload.incantationId.toString());

    const queryState = {
      queryHash,
      status: 'active',
      createdAt: Date.now(),
      lastUpdate: Date.now(),
      updateCount: 0,
      query: payload.surrealql,
      variables: {},
      dataSize: 0,
    };

    this.activeQueries.set(queryHash, queryState);

    this.addEvent('QUERY_REQUEST_INIT', {
      queryHash,
      query: payload.surrealql,
      variables: {},
    });
    this.notifyDevTools();
  }

  public onQueryUpdated(payload: any) {
    console.log('[DevTools] IncantationUpdated', payload);
    const queryHash = this.hashString(payload.incantationId.toString());

    const queryState = this.activeQueries.get(queryHash);
    if (queryState) {
      queryState.updateCount++;
      queryState.lastUpdate = Date.now();
      queryState.dataSize = Array.isArray(payload.records) ? payload.records.length : 0;
      this.activeQueries.set(queryHash, queryState);
    } else {
      console.warn('[DevTools] Received update for unknown query', queryHash);
    }

    this.addEvent('QUERY_UPDATED', {
      queryHash,
      data: payload.records,
      dataHash: 0,
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
          selector: p.record_id.toString(),
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

  private addEvent(eventType: string, payload: any) {
    this.eventsHistory.push({
      id: this.eventIdCounter++,
      timestamp: Date.now(),
      eventType,
      payload,
    });
    if (this.eventsHistory.length > 100) this.eventsHistory.shift();
  }

  private getState() {
    return {
      eventsHistory: [...this.eventsHistory],
      activeQueries: Object.fromEntries(this.activeQueries),
      auth: { authenticated: false }, // TODO: Hook up auth
      version: this.version,
      database: {
        tables: this.schema.tables.map((t) => t.name),
        tableData: {},
      },
    };
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

  private serializeForDevTools(data: any): any {
    if (data === null || data === undefined) {
      return data;
    }

    if (data instanceof RecordId) {
      return data.toString();
    }

    if (data instanceof Date) {
      return data.toISOString();
    }

    if (Array.isArray(data)) {
      return data.map((item) => this.serializeForDevTools(item));
    }

    if (typeof data === 'object') {
      const result: Record<string, any> = {};
      for (const key in data) {
        if (Object.prototype.hasOwnProperty.call(data, key)) {
          result[key] = this.serializeForDevTools(data[key]);
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
            const result = await this.databaseService.query<Record<string, unknown>[]>(
              `SELECT * FROM ${tableName}`
            );

            let records = result;
            // Check if result is double-wrapped (SurrealJS behavior: [[records]])
            if (Array.isArray(result) && Array.isArray(result[0])) {
              records = result[0] as any;
            } else if (!Array.isArray(result)) {
              records = [] as any;
            }

            return this.serializeForDevTools(records) || [];
          } catch (e) {
            this.logger.error({ err: e }, 'Failed to get table data');
            return [];
          }
        },
        updateTableRow: async (
          tableName: string,
          recordId: string,
          updates: Record<string, unknown>
        ) => {
          try {
            // Ensure updates is mapped correctly for bindings
            // const setClause = Object.keys(updates).map(k => `${k} = $val_${k}`).join(', ');
            // But simplified: UPDATE recordId MERGE $updates
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
