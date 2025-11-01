import {
  ColumnSchema,
  FinalQuery,
  InnerQuery,
  QueryInfo,
  SchemaStructure,
  TableModel,
} from "@spooky/query-builder";
import { SyncedDb } from "..";
import { LiveMessage, Uuid } from "surrealdb";

export class Executer<Schema extends SchemaStructure> {
  private _queries: Map<number, InnerQuery<any, boolean>[]> = new Map();

  constructor(private readonly db: SyncedDb<Schema>) {}

  private addQuery(queryKey: number, query: InnerQuery<any, boolean>) {
    if (!this._queries.has(queryKey)) {
      this._queries.set(queryKey, []);
    }
    this._queries.get(queryKey)!.push(query);
  }

  private removeQuery(queryKey: number, query: InnerQuery<any, boolean>) {
    if (this.queryExists(queryKey)) {
      this._queries
        .get(queryKey)!
        .splice(this._queries.get(queryKey)!.indexOf(query), 1);
    }
  }

  private queryExists(queryKey: number): boolean {
    return this._queries.has(queryKey);
  }

  async run<T extends { columns: Record<string, ColumnSchema> }>(
    query: InnerQuery<T, boolean>
  ): Promise<void> {
    if (this.queryExists(query.hash)) return;
    this.addQuery(query.hash, query);
    await this.hydrateLocal(query);
    await this.subscribeRemote(query);
  }

  private async hydrateLocal<
    T extends { columns: Record<string, ColumnSchema> }
  >(query: InnerQuery<T, boolean>): Promise<void> {
    const { query: selectQuery, vars: selectQueryVars } = query.selectQuery();
    const localQuery = this.db.queryLocal<TableModel<T>>(
      selectQuery,
      selectQueryVars
    );
    localQuery.collect().then(([data]) => {
      console.log("[Executer.hydrateLocal] Local data:", data);
      query.setData(data);
    });
  }

  private async subscribeRemote<
    T extends { columns: Record<string, ColumnSchema> }
  >(query: InnerQuery<T, boolean>): Promise<void> {
    const remoteDb = this.db.getRemote();
    if (!remoteDb) {
      return;
    }

    const { query: selectLiveQuery, vars: selectLiveQueryVars } =
      query.selectLiveQuery();
    const [liveUuid] = await remoteDb
      .query(selectLiveQuery, selectLiveQueryVars)
      .collect<[Uuid]>();

    const subscription = await remoteDb.liveOf(liveUuid);
    subscription.subscribe(async (event: LiveMessage) =>
      this.handleRemoteUpdate<T>(query, event)
    );
  }

  private async handleRemoteUpdate<
    T extends { columns: Record<string, ColumnSchema> }
  >(query: InnerQuery<T, boolean>, event: LiveMessage): Promise<void> {
    console.log("[Executer.handleRemoteUpdate] Event received:", event);

    switch (event.action) {
      case "CREATE":
        query.setData([...query.data, event.value as TableModel<T>]);
        break;
      case "UPDATE":
        query.setData(
          query.data.map((item) =>
            item.id === event.value.id ? (event.value as TableModel<T>) : item
          )
        );
        break;
      case "DELETE":
        query.setData(query.data.filter((item) => item.id !== event.value.id));
        break;
      default:
        console.warn(
          `[Executer.handleRemoteUpdate] Unknown event action: ${event.action}`
        );
        break;
    }
  }
}
