import { LiveMessage, LiveSubscription, Surreal, Table, Uuid } from "surrealdb";

/**
 * Represents a tracked live query on the remote server
 */
interface TrackedLiveQuery {
  queryKey: string;
  subscription: LiveSubscription;
  query: string;
  vars?: Record<string, unknown>;
  refCount: number;
  affectedTables: Set<string>;
}

/**
 * Syncer manages live queries on the remote server and syncs changes to local cache
 * Key responsibilities:
 * 1. Track and deduplicate live queries to the remote server
 * 2. Update local cache when remote data changes
 * 3. Notify all affected queries when data changes
 */
export class Syncer {
  private liveQueries: Map<string, TrackedLiveQuery> = new Map();
  private tableToQueryKeys: Map<string, Set<string>> = new Map();
  private queryListeners: Map<string, Set<() => void>> = new Map();
  private isInitialized = false;

  constructor(
    private localDb: Surreal,
    private remoteDb: Surreal,
    private tables: Table[]
  ) {
    this.localDb = localDb;
    this.remoteDb = remoteDb;
  }

  async init(): Promise<void> {
    if (this.isInitialized) {
      return;
    }
    this.isInitialized = true;
    console.log("[Syncer] Initialized");
  }

  /**
   * Subscribe to a live query on the remote server
   * If the same query already exists, it increases the ref count
   * @returns A function to unsubscribe from the live query
   */
  async subscribeLiveQuery(
    query: string,
    vars: Record<string, unknown> | undefined,
    affectedTables: string[],
    onUpdate: () => void
  ): Promise<() => void> {
    const queryKey = this.getQueryKey(query, vars);

    // Add the listener
    if (!this.queryListeners.has(queryKey)) {
      this.queryListeners.set(queryKey, new Set());
    }
    this.queryListeners.get(queryKey)!.add(onUpdate);

    // If query already exists, just increase ref count
    if (this.liveQueries.has(queryKey)) {
      const tracked = this.liveQueries.get(queryKey)!;
      tracked.refCount++;
      console.log(
        `[Syncer] Reusing live query (refCount: ${tracked.refCount}):`,
        queryKey
      );
      return () => this.unsubscribeLiveQuery(queryKey, onUpdate);
    }

    // Create new live query on remote
    console.log("[Syncer] Creating new live query:", queryKey);
    try {
      // Execute LIVE SELECT on remote
      const liveQuery = query.replace("SELECT", "LIVE SELECT");
      const [queryId] = (await this.remoteDb
        .query(liveQuery, vars)
        .collect()) as unknown as [Uuid];

      const subscription = await this.remoteDb.liveOf(queryId);

      // Track the live query
      const trackedQuery: TrackedLiveQuery = {
        queryKey,
        subscription,
        query,
        vars,
        refCount: 1,
        affectedTables: new Set(affectedTables),
      };

      this.liveQueries.set(queryKey, trackedQuery);

      // Map tables to query keys for efficient lookups
      for (const table of affectedTables) {
        if (!this.tableToQueryKeys.has(table)) {
          this.tableToQueryKeys.set(table, new Set());
        }
        this.tableToQueryKeys.get(table)!.add(queryKey);
      }

      // Subscribe to updates
      subscription.subscribe(async (event) => {
        await this.handleRemoteUpdate(queryKey, event);
      });

      console.log(`[Syncer] Live query started (refCount: 1):`, queryKey);
    } catch (error) {
      console.error("[Syncer] Failed to create live query:", error);
      // Clean up listeners on error
      this.queryListeners.get(queryKey)?.delete(onUpdate);
      throw error;
    }

    return () => this.unsubscribeLiveQuery(queryKey, onUpdate);
  }

  /**
   * Unsubscribe from a live query
   * Decreases ref count and kills the query if no more subscribers
   */
  private unsubscribeLiveQuery(queryKey: string, listener: () => void): void {
    // Remove the listener
    const listeners = this.queryListeners.get(queryKey);
    if (listeners) {
      listeners.delete(listener);
      if (listeners.size === 0) {
        this.queryListeners.delete(queryKey);
      }
    }

    const tracked = this.liveQueries.get(queryKey);
    if (!tracked) return;

    tracked.refCount--;
    console.log(
      `[Syncer] Unsubscribed from live query (refCount: ${tracked.refCount}):`,
      queryKey
    );

    // If no more subscribers, kill the live query
    if (tracked.refCount <= 0) {
      console.log("[Syncer] Killing live query:", queryKey);
      tracked.subscription.kill();
      this.liveQueries.delete(queryKey);

      // Clean up table mappings
      for (const table of tracked.affectedTables) {
        const queryKeys = this.tableToQueryKeys.get(table);
        if (queryKeys) {
          queryKeys.delete(queryKey);
          if (queryKeys.size === 0) {
            this.tableToQueryKeys.delete(table);
          }
        }
      }
    }
  }

  /**
   * Handle updates from remote live queries and sync to local cache
   */
  private async handleRemoteUpdate(
    queryKey: string,
    event: LiveMessage
  ): Promise<void> {
    try {
      console.log(`[Syncer] Remote update for query:`, queryKey, event);

      const tracked = this.liveQueries.get(queryKey);
      if (!tracked) return;

      // Extract the record ID from the event
      const recordValue = event.value as any;
      const recordId = recordValue?.id;

      if (!recordId) {
        console.warn("[Syncer] Event value has no id:", event);
        return;
      }

      // Update local cache based on the action
      if (event.action === "CREATE") {
        const tableName = recordId.tb;
        await this.localDb.insert(tableName, recordValue);
      } else if (event.action === "UPDATE") {
        await this.localDb.update(recordId).merge(recordValue);
      } else if (event.action === "DELETE") {
        await this.localDb.delete(recordId);
      }

      // Notify all listeners of this query
      const listeners = this.queryListeners.get(queryKey);
      if (listeners) {
        for (const listener of listeners) {
          try {
            listener();
          } catch (error) {
            console.error("[Syncer] Error in query listener:", error);
          }
        }
      }
    } catch (error) {
      console.error(
        `[Syncer] Error processing remote update for query ${queryKey}:`,
        error
      );
    }
  }

  /**
   * Generate a unique key for a query based on SQL and variables
   */
  private getQueryKey(
    query: string,
    vars?: Record<string, unknown>
  ): string {
    const varsStr = vars ? JSON.stringify(vars) : "";
    return `${query}|${varsStr}`;
  }

  async destroy(): Promise<void> {
    console.log("[Syncer] Destroying all live queries");
    // Stop all live queries
    for (const [queryKey, tracked] of this.liveQueries) {
      try {
        tracked.subscription.kill();
      } catch (error) {
        console.error(
          `[Syncer] Error stopping live query ${queryKey}:`,
          error
        );
      }
    }
    this.liveQueries.clear();
    this.tableToQueryKeys.clear();
    this.queryListeners.clear();
    this.isInitialized = false;
  }

  /**
   * Get syncer instance if available
   */
  isActive(): boolean {
    return this.isInitialized;
  }
}
