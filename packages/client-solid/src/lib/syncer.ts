import { LiveMessage, LiveSubscription, Surreal, Table } from "surrealdb";

export class Syncer {
  private liveQueries: Map<Table, LiveSubscription> = new Map();

  constructor(
    private localDb: Surreal,
    private remoteDb: Surreal,
    private tables: Table[]
  ) {
    this.localDb = localDb;
    this.remoteDb = remoteDb;
  }

  async startSyncTable(table: Table) {
    if (this.liveQueries.has(table)) {
      return;
    }
    const liveQuery = await this.localDb.live(table).diff();
    liveQuery.subscribe((event) => this.onLiveQueryUpdate(table, event));
    this.liveQueries.set(table, liveQuery);
  }

  async stopSyncTable(table: Table) {
    if (!this.liveQueries.has(table)) {
      return;
    }
    this.liveQueries.get(table)?.kill();
    this.liveQueries.delete(table);
  }

  private onLiveQueryUpdate(table: Table, event: LiveMessage) {
    console.log(table, event);
  }
}
