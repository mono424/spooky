import { LiveMessage, LiveSubscription, Surreal, Table } from "surrealdb";

export class Syncer {
  private liveQueries: Map<Table, LiveSubscription> = new Map();
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

    // Initialize sync for all tables with delays to prevent race conditions
    for (let i = 0; i < this.tables.length; i++) {
      const table = this.tables[i];
      await this.startSyncTable(table);

      // Add a small delay between table syncs to prevent circular reference issues
      if (i < this.tables.length - 1) {
        await new Promise((resolve) => setTimeout(resolve, 100));
      }
    }

    this.isInitialized = true;
  }

  async startSyncTable(table: Table, retryCount = 0): Promise<void> {
    if (this.liveQueries.has(table)) {
      return;
    }

    const maxRetries = 3;
    const retryDelay = 1000 * Math.pow(2, retryCount); // Exponential backoff

    try {
      // Add a small delay before starting the live query to prevent race conditions
      await new Promise((resolve) => setTimeout(resolve, 50));

      // Check if the table exists and is accessible before starting live query
      try {
        await this.localDb.query(`SELECT * FROM ${table.name} LIMIT 1`);
      } catch (tableError) {
        console.warn(
          `[Syncer] Table ${table.name} may not be ready yet, waiting...`
        );
        await new Promise((resolve) => setTimeout(resolve, 200));
      }

      const liveQuery = await this.localDb.live(table).diff();
      liveQuery.subscribe((event) => this.onLiveQueryUpdate(table, event));
      this.liveQueries.set(table, liveQuery);
      console.log(`[Syncer] Successfully started sync for table ${table.name}`);
    } catch (error) {
      console.error(
        `[Syncer] Failed to start sync for table ${table.name} (attempt ${
          retryCount + 1
        }/${maxRetries + 1}):`,
        error
      );

      // Retry with exponential backoff if we haven't exceeded max retries
      if (retryCount < maxRetries) {
        console.log(
          `[Syncer] Retrying sync for table ${table.name} in ${retryDelay}ms...`
        );
        await new Promise((resolve) => setTimeout(resolve, retryDelay));
        return this.startSyncTable(table, retryCount + 1);
      } else {
        console.error(
          `[Syncer] Max retries exceeded for table ${table.name}, skipping sync`
        );
        // Don't throw the error, just log it and continue with other tables
      }
    }
  }

  async stopSyncTable(table: Table) {
    if (!this.liveQueries.has(table)) {
      return;
    }
    this.liveQueries.get(table)?.kill();
    this.liveQueries.delete(table);
  }

  async destroy(): Promise<void> {
    // Stop all live queries
    for (const [table, liveQuery] of this.liveQueries) {
      try {
        liveQuery.kill();
      } catch (error) {
        console.error(
          `[Syncer] Error stopping sync for table ${table.name}:`,
          error
        );
      }
    }
    this.liveQueries.clear();
    this.isInitialized = false;
  }

  private onLiveQueryUpdate(table: Table, event: LiveMessage) {
    try {
      console.log(`[Syncer] Live update for ${table.name}:`, event);
      // TODO: Implement actual sync logic here
      // This is where we would sync changes between local and remote databases
    } catch (error) {
      console.error(
        `[Syncer] Error processing live update for ${table.name}:`,
        error
      );
      // Don't rethrow the error to prevent breaking the live query
    }
  }
}
