import {
  FinalQuery,
  InnerQuery,
  QueryInfo,
  SchemaStructure,
} from "@spooky/query-builder";
import { SyncedDb } from "..";
import { LiveMessage, Uuid } from "surrealdb";

export class Executer<Schema extends SchemaStructure> {
  private _queries: Map<number, InnerQuery<any>[]> = new Map();

  constructor(private readonly db: SyncedDb<Schema>) {}

  private addQuery(queryKey: number, query: InnerQuery<any>) {
    if (!this._queries.has(queryKey)) {
      this._queries.set(queryKey, []);
    }
    this._queries.get(queryKey)!.push(query);
  }

  private removeQuery(queryKey: number, query: InnerQuery<any>) {
    if (this.queryExists(queryKey)) {
      this._queries
        .get(queryKey)!
        .splice(this._queries.get(queryKey)!.indexOf(query), 1);
    }
  }

  private queryExists(queryKey: number): boolean {
    return this._queries.has(queryKey);
  }

  async run<T>(query: InnerQuery<T>): Promise<void> {
    if (this.queryExists(query.hash)) return;
    this.addQuery(query.hash, query);
    await this.hydrateLocal(query);
    await this.subscribeRemote(query);
  }

  private async hydrateLocal<T>(query: InnerQuery<any>): Promise<void> {
    const { query: selectQuery, vars: selectQueryVars } = query.selectQuery();
    const localQuery = this.db.queryLocal<T>(selectQuery, selectQueryVars);
    localQuery.collect().then(([data]) => {
      query.setData(data);
    });
  }

  private async subscribeRemote<T>(query: InnerQuery<any>): Promise<void> {
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

  private async handleRemoteUpdate<T>(
    query: InnerQuery<T>,
    event: LiveMessage
  ): Promise<void> {
    console.log("[Executer.handleRemoteUpdate] Event received:", event);
    await query.setData(event.value as T);
  }
}
