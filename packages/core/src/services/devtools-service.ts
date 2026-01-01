import { LocalDatabaseService } from './database/index.js';
import { Logger } from './logger.js';
import { MutationEventSystem, MutationEventTypes } from './mutation/events.js';
import { QueryEventSystem, QueryEventTypes } from './query/events.js';
import { SchemaStructure } from '@spooky/query-builder';
import { RecordId } from 'surrealdb';

// DevTools interfaces (matching extension expectations)
export interface DevToolsEvent {
  id: number;
  timestamp: number;
  eventType: string;
  payload: any;
}

export class DevToolsService {
  private eventsHistory: DevToolsEvent[] = [];
  private eventIdCounter = 0;
  private version = '1.0.0'; // TODO: Get from package.json

  constructor(
    private mutationEvents: MutationEventSystem,
    private queryEvents: QueryEventSystem,
    private databaseService: LocalDatabaseService,
    private logger: Logger,
    private schema: SchemaStructure
  ) {
    this.setupEventSubscriptions();
    this.exposeToWindow();
    this.logger.debug('[DevTools] Service initialized');
  }

  private setupEventSubscriptions() {
    // Query Events
    this.queryEvents.subscribe(QueryEventTypes.IncantationInitialized, (event) => {
      // Map incantationId hash to number if possible, or use string hash
      // Devtools expects number queryHash. If existing is string, we might need to hash it or change devtools.
      // For now, let's try to parse integer if it's numeric, or hash the string.
      const queryHash = this.hashString(event.payload.incantationId.toString());

      this.addEvent('QUERY_REQUEST_INIT', {
        queryHash,
        query: event.payload.surrealql,
        variables: {}, // Core doesn't seem to expose vars in this event yet
      });
      this.notifyDevTools();
    });

    this.queryEvents.subscribe(QueryEventTypes.IncantationUpdated, (event) => {
      const queryHash = this.hashString(event.payload.incantationId.toString());
      this.addEvent('QUERY_UPDATED', {
        queryHash,
        data: event.payload.records,
        dataHash: 0, // Placeholder
      });
      this.notifyDevTools();
    });

    // Mutation Events
    this.mutationEvents.subscribe(MutationEventTypes.MutationCreated, (event) => {
      // Flatten inputs
      const payloads = event.payload;
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
    });
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
      activeQueries: {}, // TODO: Track active queries if needed
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
            // Returns the first statement result as T
            const result = await this.databaseService.query<Record<string, unknown>[]>(
              `SELECT * FROM ${tableName}`
            );
            return result || [];
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
