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
import { AuthService } from '../auth/index.js';
import { AuthEventTypes } from '../auth/events.js';

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
    private authService: AuthService<SchemaStructure>,
    private queryManager?: QueryManager<any>
  ) {
    // this.setupEventSubscriptions(); // REMOVED
    this.exposeToWindow();
    this.syncInitialState();

    // Subscribe to auth events
    this.authService.eventSystem.subscribe(AuthEventTypes.AuthStateChanged, () => {
      this.notifyDevTools();
    });

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
          data: q.records,
          localHash: q.localHash,
          localArray: q.localArray,
          remoteHash: q.remoteHash,
          remoteArray: q.remoteArray,
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
      localHash: payload.localHash,
      localArray: payload.localArray,
      remoteHash: payload.remoteHash,
      remoteArray: payload.remoteArray,
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
    console.log('[DevTools] IncantationUpdated', {
      id: payload.incantationId?.toString(),
      localHash: payload.localHash,
      remoteHash: payload.remoteHash,
      localArray: payload.localArray ? 'PRESENT' : 'MISSING',
      remoteArray: payload.remoteArray ? 'PRESENT' : 'MISSING',
    });
    const queryHash = this.hashString(payload.incantationId.toString());

    const queryState = this.activeQueries.get(queryHash);
    if (queryState) {
      queryState.updateCount++;
      queryState.lastUpdate = Date.now();
      queryState.dataSize = Array.isArray(payload.records) ? payload.records.length : 0;
      queryState.data = payload.records;

      // Update local state from payload
      if (payload.localArray !== undefined) {
        queryState.localArray = payload.localArray;
        // Optionally update hash if not provided (though usually hash comes with array)
      }
      if (payload.localHash !== undefined) {
        queryState.localHash = payload.localHash;
      }

      // Update remote state
      if (payload.remoteHash) queryState.remoteHash = payload.remoteHash;
      if (payload.remoteArray) queryState.remoteArray = payload.remoteArray;

      this.activeQueries.set(queryHash, queryState);
      console.log('[DevTools] Updated QueryState:', {
        localHash: queryState.localHash,
        remoteHash: queryState.remoteHash,
        localArraySize: queryState.localArray?.length ?? 'MISSING',
        remoteArraySize: queryState.remoteArray?.length ?? 'MISSING',
      });
    } else {
      console.warn('[DevTools] Received update for unknown query', queryHash);
    }

    this.addEvent('QUERY_UPDATED', {
      queryHash,
      query: queryState?.query,
      data: payload.records,
      dataHash: 0,
    });
    this.notifyDevTools();
  }

  public onStreamUpdate(payload: any) {
    console.log('[DevTools] StreamUpdate', payload);
    this.addEvent('STREAM_UPDATE', {
      updates: payload,
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
    return {
      eventsHistory: [...this.eventsHistory],
      activeQueries: Object.fromEntries(this.activeQueries),
      auth: {
        authenticated: this.authService.isAuthenticated,
        userId: this.authService.currentUser?.id,
      },
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
