import {
  SpookyEventSystem,
  AuthEventTypes,
  GlobalQueryEventTypes,
  MutationEventTypes,
} from './spooky-event-system.js';
import { Logger } from './logger.js';
import { DatabaseService } from './database.js';
import { SchemaStructure } from '@spooky/query-builder';
import { QueryManagerService } from './query-manager.js';
import { RecordId } from 'surrealdb';

/**
 * Event entry in the history
 */
export interface DevToolsEvent {
  id: number;
  timestamp: number;
  eventType: string;
  payload: any;
}

/**
 * Active query tracking information
 */
export interface ActiveQuery {
  queryHash: number;
  status: 'initializing' | 'active' | 'updating' | 'destroyed';
  createdAt: number;
  lastUpdate: number;
  updateCount: number;
  dataSize?: number;
  query?: string;
  variables?: Record<string, unknown>;
  listenerCount?: number;
  connectedQueries?: number[];
  dataHash?: number;
  data?: any;
}

/**
 * Authentication state
 */
export interface AuthState {
  authenticated: boolean;
  userId?: string;
  timestamp?: number;
}

/**
 * Database state
 */
export interface DatabaseState {
  tables: string[];
  tableData: Record<string, Record<string, unknown>[]>;
}

/**
 * Complete DevTools state
 */
export interface DevToolsState {
  eventsHistory: DevToolsEvent[];
  activeQueries: Record<number, ActiveQuery>;
  auth: AuthState;
  version: string;
  database?: DatabaseState;
}

/**
 * DevTools Service Configuration
 */
export interface DevToolsServiceConfig {
  maxEvents?: number; // Maximum events to keep in history (default: 100)
  enabled?: boolean; // Enable/disable devtools (default: true)
  version?: string; // Spooky version
}

/**
 * DevTools Service - Tracks all Spooky events and exposes state to DevTools
 */
export class DevToolsService {
  private eventsHistory: DevToolsEvent[] = [];
  private activeQueries: Map<number, ActiveQuery> = new Map();
  private auth: AuthState = { authenticated: false };
  private eventIdCounter = 0;
  private maxEvents: number;
  private enabled: boolean;
  private version: string;
  private subscriptionMap: Map<number, Set<number>> = new Map(); // queryHash -> Set of subscriber queryHashes

  constructor(
    private eventSystem: SpookyEventSystem,
    private logger: Logger,
    private databaseService: DatabaseService,
    private schema: SchemaStructure,
    // queryManager reserved for future subscription tracking implementation
    // eslint-disable-next-line @typescript-eslint/no-unused-vars
    private _queryManager?: QueryManagerService<SchemaStructure>,
    config: DevToolsServiceConfig = {}
  ) {
    this.maxEvents = config.maxEvents ?? 100;
    this.enabled = config.enabled ?? true;
    this.version = config.version ?? 'unknown';

    console.log('[DevTools] Service constructor called', {
      enabled: this.enabled,
      version: this.version,
    });

    if (this.enabled) {
      this.setupEventSubscriptions();
      this.exposeToWindow();
      this.logger.debug('[DevTools] Service initialized');
      console.log('[DevTools] Service fully initialized, window.__SPOOKY__ exposed');
    }
  }

  /**
   * Subscribe to all Spooky events
   */
  private setupEventSubscriptions(): void {
    // Auth events
    this.eventSystem.subscribe(AuthEventTypes.Authenticated, (event) => {
      this.auth = {
        authenticated: true,
        userId: event.payload.userId.toString(),
        timestamp: Date.now(),
      };
      this.addEvent(AuthEventTypes.Authenticated, event.payload);
      this.notifyDevTools();
    });

    this.eventSystem.subscribe(AuthEventTypes.Deauthenticated, (event) => {
      this.auth = {
        authenticated: false,
        timestamp: Date.now(),
      };
      this.addEvent(AuthEventTypes.Deauthenticated, {});
      this.notifyDevTools();
    });

    // Query lifecycle events
    this.eventSystem.subscribe(GlobalQueryEventTypes.RequestInit, (event) => {
      const queryHash = event.payload.queryHash;
      const activeQuery: ActiveQuery = {
        queryHash,
        status: 'initializing',
        createdAt: Date.now(),
        lastUpdate: Date.now(),
        updateCount: 0,
      };

      if (event.payload.query !== undefined) {
        activeQuery.query = event.payload.query;
      }
      if (event.payload.variables !== undefined) {
        activeQuery.variables = event.payload.variables;
      }

      this.activeQueries.set(queryHash, activeQuery);
      this.addEvent(GlobalQueryEventTypes.RequestInit, event.payload);
      this.notifyDevTools();
    });

    this.eventSystem.subscribe(GlobalQueryEventTypes.Updated, (event) => {
      const queryHash = event.payload.queryHash;
      const query = this.activeQueries.get(queryHash);
      if (query) {
        // Log update for debugging
        console.log(`[DevTools] Query Updated ${queryHash}`, {
          dataLength: event.payload.data?.length,
          dataSample: event.payload.data?.slice(0, 1),
        });

        query.status = 'active';
        query.lastUpdate = Date.now();
        query.updateCount++;
        query.dataSize = event.payload.data?.length ?? 0;
        query.dataHash = event.payload.dataHash;
        query.data = this.sanitizeData(event.payload.data);

        console.log(`[DevTools] Query Data set:`, query.data);
      }
      this.addEvent(GlobalQueryEventTypes.Updated, {
        queryHash: event.payload.queryHash,
        dataSize: event.payload.data?.length ?? 0,
        dataHash: event.payload.dataHash,
      });
      this.notifyDevTools();
    });

    this.eventSystem.subscribe(GlobalQueryEventTypes.RemoteUpdate, (event) => {
      const queryHash = event.payload.queryHash;
      const query = this.activeQueries.get(queryHash);
      if (query) {
        query.status = 'updating';
        query.lastUpdate = Date.now();
        query.updateCount++;
        query.dataSize = event.payload.data?.length ?? 0;
        query.data = this.sanitizeData(event.payload.data);
      }
      this.addEvent(GlobalQueryEventTypes.RemoteUpdate, {
        queryHash: event.payload.queryHash,
        dataSize: event.payload.data?.length ?? 0,
      });
      this.notifyDevTools();
    });

    this.eventSystem.subscribe(GlobalQueryEventTypes.RemoteLiveUpdate, (event) => {
      const queryHash = event.payload.queryHash;
      const query = this.activeQueries.get(queryHash);
      if (query) {
        query.lastUpdate = Date.now();
      }
      this.addEvent(GlobalQueryEventTypes.RemoteLiveUpdate, {
        queryHash: event.payload.queryHash,
        action: event.payload.action,
      });
      this.notifyDevTools();
    });

    this.eventSystem.subscribe(GlobalQueryEventTypes.Destroyed, (event) => {
      const queryHash = event.payload.queryHash;
      const query = this.activeQueries.get(queryHash);
      if (query) {
        query.status = 'destroyed';
        query.lastUpdate = Date.now();
      }
      this.addEvent(GlobalQueryEventTypes.Destroyed, event.payload);
      // Remove from active queries after a delay to allow DevTools to see it
      setTimeout(() => {
        this.activeQueries.delete(queryHash);
      }, 5000);
      this.notifyDevTools();
    });

    this.eventSystem.subscribe(GlobalQueryEventTypes.SubqueryUpdated, (event) => {
      this.addEvent(GlobalQueryEventTypes.SubqueryUpdated, event.payload);
      this.notifyDevTools();
    });

    this.eventSystem.subscribe(MutationEventTypes.RequestExecution, (event) => {
      // Sanitize payload to avoid DataCloneError with InnerQuery objects
      const sanitizedPayload = this.sanitizePayload(event.payload);
      this.addEvent(MutationEventTypes.RequestExecution, sanitizedPayload);
      this.notifyDevTools();
    });

    this.eventSystem.subscribe(GlobalQueryEventTypes.RequestTableQueryRefresh, (event) => {
      this.addEvent(GlobalQueryEventTypes.RequestTableQueryRefresh, event.payload);
      this.notifyDevTools();
    });
  }

  /**
   * Add an event to the history
   */
  private addEvent(eventType: string, payload: any): void {
    const event: DevToolsEvent = {
      id: this.eventIdCounter++,
      timestamp: Date.now(),
      eventType,
      payload,
    };

    this.eventsHistory.push(event);

    // Keep only the last N events
    if (this.eventsHistory.length > this.maxEvents) {
      this.eventsHistory.shift();
    }

    console.log(`[DevTools] Event added: ${eventType}`, payload);
    this.logger.debug(`[DevTools] Event added: ${eventType}`, payload);
  }

  /**
   * Sanitize payload to ensure it can be sent via postMessage
   */
  private sanitizePayload(payload: any): any {
    if (!payload) return payload;

    // Handle Mutation payload
    if (payload.mutation && payload.mutation.selector) {
      const mutation = { ...payload.mutation };

      // Check if selector is an InnerQuery (has selectQuery property)
      if (
        typeof mutation.selector === 'object' &&
        mutation.selector !== null &&
        'selectQuery' in mutation.selector
      ) {
        // Convert InnerQuery to a serializable object
        mutation.selector = {
          tableName: mutation.selector.tableName,
          query: mutation.selector.selectQuery.query,
          vars: mutation.selector.selectQuery.vars,
          isInnerQuery: true,
        };
      }

      // Also sanitize data in mutation to handle Dates and RecordIds
      if (mutation.data) {
        mutation.data = this.sanitizeData(mutation.data);
      }
      if (mutation.patches) {
        mutation.patches = this.sanitizeData(mutation.patches);
      }

      return { ...payload, mutation };
    }

    return this.sanitizeData(payload);
  }

  /**
   * Sanitize data to ensure it can be sent via postMessage
   * Converts RecordId objects to strings
   */
  private sanitizeData(data: any): any {
    if (!data) return data;

    if (Array.isArray(data)) {
      return data.map((item) => this.sanitizeData(item));
    }

    if (data instanceof RecordId) {
      return data.toString();
    }

    // Handle RecordId-like objects (e.g. from different surrealdb versions)
    if (
      typeof data === 'object' &&
      data !== null &&
      'tb' in data &&
      'id' in data &&
      Object.keys(data).length === 2
    ) {
      return `${data.tb}:${data.id}`;
    }

    if (typeof data === 'object' && data !== null) {
      // Handle Date objects
      if (data instanceof Date) {
        return data.toISOString();
      }

      const sanitized: any = {};
      for (const key in data) {
        sanitized[key] = this.sanitizeData(data[key]);
      }
      return sanitized;
    }

    return data;
  }

  /**
   * Get database tables from schema
   */
  private getTables(): string[] {
    return this.schema.tables.map((table) => table.name);
  }

  /**
   * Get table data from local database
   */
  private async getTableData(tableName: string): Promise<Record<string, unknown>[]> {
    try {
      const query = `SELECT * FROM ${tableName}`;
      const result = await this.databaseService.queryLocal<Record<string, unknown>[]>(query);
      return result || [];
    } catch (error) {
      this.logger.error(`[DevTools] Failed to fetch table data for ${tableName}`, error);
      return [];
    }
  }

  /**
   * Update a row in the database
   */
  private async updateTableRow(
    tableName: string,
    recordId: string,
    updates: Record<string, unknown>
  ): Promise<{ success: boolean; error?: string }> {
    try {
      // Build the UPDATE query with MERGE to update specific fields
      const query = `UPDATE ${recordId} MERGE $updates`;
      await this.databaseService.queryLocal(query, { updates });
      this.logger.debug(`[DevTools] Updated row ${recordId} in ${tableName}`, updates);
      return { success: true };
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      this.logger.error(`[DevTools] Failed to update row ${recordId} in ${tableName}`, error);
      return { success: false, error: errorMessage };
    }
  }

  /**
   * Delete a row from the database
   */
  private async deleteTableRow(
    tableName: string,
    recordId: string
  ): Promise<{ success: boolean; error?: string }> {
    try {
      const query = `DELETE ${recordId}`;
      await this.databaseService.queryLocal(query);
      this.logger.debug(`[DevTools] Deleted row ${recordId} from ${tableName}`);
      return { success: true };
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      this.logger.error(`[DevTools] Failed to delete row ${recordId} from ${tableName}`, error);
      return { success: false, error: errorMessage };
    }
  }

  /**
   * Update subscription tracking for queries
   */
  private updateSubscriptionTracking(): void {
    // Reset subscription tracking
    this.subscriptionMap.clear();

    // For now, we'll track this manually when we have access to QueryManagerService
    // This is a placeholder that can be enhanced to track actual subscriptions
    // by accessing QueryManagerService's internal state or by emitting subscription events
    // TODO: Use this._queryManager to get actual subscription information

    // Update listener counts and connected queries for each active query
    for (const [queryHash, query] of this.activeQueries.entries()) {
      const subscribers = this.subscriptionMap.get(queryHash) || new Set<number>();
      query.listenerCount = subscribers.size;
      query.connectedQueries = Array.from(subscribers);
    }

    // Reference queryManager to avoid unused variable warning
    // Will be used when implementing full subscription tracking
    void this._queryManager;
  }

  /**
   * Track a subscription from one query to another
   */
  public trackSubscription(subscriberHash: number, targetHash: number): void {
    if (!this.subscriptionMap.has(targetHash)) {
      this.subscriptionMap.set(targetHash, new Set());
    }
    this.subscriptionMap.get(targetHash)!.add(subscriberHash);
    this.updateSubscriptionTracking();
  }

  /**
   * Track an unsubscription
   */
  public trackUnsubscription(subscriberHash: number, targetHash: number): void {
    const subscribers = this.subscriptionMap.get(targetHash);
    if (subscribers) {
      subscribers.delete(subscriberHash);
      if (subscribers.size === 0) {
        this.subscriptionMap.delete(targetHash);
      }
    }
    this.updateSubscriptionTracking();
  }

  /**
   * Get the current DevTools state
   */
  public getState(): DevToolsState {
    this.updateSubscriptionTracking();
    return {
      eventsHistory: [...this.eventsHistory],
      activeQueries: Object.fromEntries(this.activeQueries),
      auth: { ...this.auth },
      version: this.version,
      database: {
        tables: this.getTables(),
        tableData: {},
      },
    };
  }

  /**
   * Notify DevTools of state changes via window.postMessage
   */
  private notifyDevTools(): void {
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

  /**
   * Expose DevTools API to window object
   */
  private exposeToWindow(): void {
    if (typeof window !== 'undefined') {
      (window as any).__SPOOKY__ = {
        version: this.version,
        getState: () => this.getState(),
        clearHistory: () => {
          this.eventsHistory = [];
          this.notifyDevTools();
        },
        getTableData: async (tableName: string) => {
          return await this.getTableData(tableName);
        },
        updateTableRow: async (
          tableName: string,
          recordId: string,
          updates: Record<string, unknown>
        ) => {
          return await this.updateTableRow(tableName, recordId, updates);
        },
        deleteTableRow: async (tableName: string, recordId: string) => {
          return await this.deleteTableRow(tableName, recordId);
        },
      };

      this.logger.debug('[DevTools] Exposed window.__SPOOKY__ API');

      // Notify that Spooky is initialized
      window.postMessage(
        {
          type: 'SPOOKY_DETECTED',
          source: 'spooky-devtools-page',
          data: {
            version: this.version,
            detected: true,
          },
        },
        '*'
      );
    }
  }

  /**
   * Clean up resources
   */
  public destroy(): void {
    this.eventsHistory = [];
    this.activeQueries.clear();
    if (typeof window !== 'undefined') {
      delete (window as any).__SPOOKY__;
    }
  }
}

/**
 * Create a new DevTools service
 */
export function createDevToolsService(
  eventSystem: SpookyEventSystem,
  logger: Logger,
  databaseService: DatabaseService,
  schema: SchemaStructure,
  queryManager?: QueryManagerService<SchemaStructure>,
  config?: DevToolsServiceConfig
): DevToolsService {
  return new DevToolsService(eventSystem, logger, databaseService, schema, queryManager, config);
}
