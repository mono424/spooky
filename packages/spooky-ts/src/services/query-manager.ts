import {
  ColumnSchema,
  InnerQuery,
  SchemaStructure,
  TableModel,
} from "@spooky/query-builder";
import {
  AuthManagerService,
  DatabaseService,
  SpookyEventSystem,
  QueryEventTypes,
  GlobalQueryEventTypes,
} from "./index.js";
import { LiveHandler, RecordId, Uuid } from "surrealdb";
import { decodeFromSpooky } from "./converter.js";
import { Logger } from "./logger.js";
import {
  createQueryEventSystem,
  QueryEventSystem,
} from "./query-event-system.js";
import { EventSubscriptionOptions } from "src/events/index.js";

export interface Query<
  T extends { columns: Record<string, ColumnSchema> },
  IsOne extends boolean
> {
  innerQuery: InnerQuery<T, IsOne>;
  eventSystem: QueryEventSystem;
}

export interface QueryCache<S extends SchemaStructure> {
  [key: number]: Query<{ columns: S["tables"][number]["columns"] }, boolean>;
}

export class QueryManagerService<S extends SchemaStructure> {
  private cache: QueryCache<S> = {};

  constructor(
    private schema: S,
    private databaseService: DatabaseService,
    private authManager: AuthManagerService,
    private logger: Logger,
    private eventSystem: SpookyEventSystem
  ) {
    this.eventSystem.subscribe(QueryEventTypes.Updated, (event) => {
      this.cache[event.payload.queryHash].eventSystem.addEvent({
        type: QueryEventTypes.Updated,
        payload: {
          type: "local",
          data: event.payload.data,
        },
      });
      // this.cache[event.payload.queryHash].innerQuery.setData(
      //   event.payload.data as any
      // );
    });

    this.eventSystem.subscribe(GlobalQueryEventTypes.RemoteUpdate, (event) => {
      this.cache[event.payload.queryHash].eventSystem.addEvent({
        type: GlobalQueryEventTypes.Updated,
        payload: {
          type: "remote",
          data: event.payload.data,
        },
      });
      // this.cache[event.payload.queryHash].innerQuery.setData(
      //   event.payload.data as any
      // );
    });

    this.eventSystem.subscribe(
      GlobalQueryEventTypes.SubqueryUpdated,
      (event) => {
        this.queryLocalRefresh(event.payload.queryHash);
      }
    );

    this.eventSystem.subscribe(GlobalQueryEventTypes.RequestInit, (event) => {
      this.initQuery(event.payload.queryHash);
    });

    this.eventSystem.subscribe(
      GlobalQueryEventTypes.RemoteLiveUpdate,
      (event) => {
        this.handleRemoteUpdate(
          event.payload.queryHash,
          event.payload.action,
          event.payload.update
        );
      }
    );
  }

  /**
   * Custom JSON.stringify function that formats Date objects as SurrealDB date literals.
   * Dates are formatted as: d"2025-11-12T05:47:42.527106262Z"
   */
  private surrealStringify(tableName: string, value: unknown): string {
    const table = this.schema.tables.find((t) => t.name === tableName);
    if (!table) {
      throw new Error(`Table ${tableName} not found in schema`);
    }

    // Use a unique placeholder for dates to avoid conflicts
    const DATE_PLACEHOLDER = "__SURREAL_DATE__";
    const RECORDID_PLACEHOLDER = "__SURREAL_RECORDID__";

    // First pass: replace Date objects with placeholders
    const replacer = (key: string, val: unknown): unknown => {
      if (table.columns[key]?.dateTime) {
        return DATE_PLACEHOLDER + val + DATE_PLACEHOLDER;
      }
      if (table.columns[key]?.recordId) {
        return RECORDID_PLACEHOLDER + val + RECORDID_PLACEHOLDER;
      }
      return val;
    };

    // Stringify with replacer
    let jsonString = JSON.stringify(value, replacer);
    // Second pass: replace placeholders with SurrealDB date format
    // Match the placeholder pattern and replace with SurrealDB date literal
    jsonString = jsonString.replace(
      new RegExp(`"${DATE_PLACEHOLDER}([^"]+)${DATE_PLACEHOLDER}"`, "g"),
      (match, isoString) => `d"${isoString}"`
    );

    jsonString = jsonString.replace(
      new RegExp(
        `"${RECORDID_PLACEHOLDER}([^"]+)${RECORDID_PLACEHOLDER}"`,
        "g"
      ),
      (match, recordId) => `${recordId}`
    );

    return jsonString;
  }

  private async queryLocalRefresh<
    T extends { columns: Record<string, ColumnSchema> }
  >(queryHash: number): Promise<void> {
    const query = this.cache[queryHash].innerQuery;
    if (!query) {
      this.logger.error(
        "[QueryManager] Local Query Refresh - Query not found",
        {
          queryHash: queryHash,
        }
      );
      return;
    }

    this.logger.debug("[QueryManager] Local Query Refresh - Starting", {
      queryHash: query.hash,
    });

    const results = await this.databaseService.queryLocal<TableModel<T>[]>(
      query.selectQuery.query,
      query.selectQuery.vars
    );

    this.logger.debug("[QueryManager] Local Query Refresh - Done", {
      queryHash: query.hash,
      resultLength: results?.length ?? 0,
    });

    const decodedResults = results
      .map((result) => decodeFromSpooky(this.schema, query.tableName, result))
      .filter((result) => result !== undefined);

    this.eventSystem.addEvent({
      type: QueryEventTypes.Updated,
      payload: {
        queryHash: query.hash,
        data: decodedResults,
      },
    });
  }

  async refreshTableQueries(table: string): Promise<void> {
    this.logger.debug("[QueryManager] Refresh Table Queries - Starting", {
      table,
    });

    for (const query of Object.values(this.cache)) {
      if (query.innerQuery.tableName === table) {
        try {
          await this.queryLocalRefresh(query.innerQuery.hash);
        } catch (error) {
          this.logger.error("Failed to refresh query", error);
        }
      }
    }

    this.logger.debug("[QueryManager] Refresh Table Queries - Done", {
      table,
    });
  }

  private async triggerQueryUpdate<
    T extends { columns: Record<string, ColumnSchema> }
  >(queryHash: number): Promise<void> {
    const query = this.cache[queryHash];
    if (!query) {
      this.logger.error(
        "[QueryManager] Trigger Query Update - Query not found",
        {
          queryHash: queryHash,
        }
      );
      return;
    }

    const results = await this.databaseService.queryLocal<TableModel<T>[]>(
      query.innerQuery.selectQuery.query,
      query.innerQuery.selectQuery.vars
    );
    this.logger.debug(
      "[QueryManager] Remote Query Hydration - Local cache updated",
      {
        queryHash,
        resultLength: results?.length ?? 0,
      }
    );

    const decodedResults = results.map((result) =>
      decodeFromSpooky(this.schema, query.innerQuery.tableName, result)
    );

    this.eventSystem.addEvent({
      type: GlobalQueryEventTypes.RemoteUpdate,
      payload: {
        queryHash,
        data: decodedResults,
      },
    });
  }

  private async queryRemoteHydration<
    T extends { columns: Record<string, ColumnSchema> }
  >(query: InnerQuery<T, boolean>): Promise<void> {
    this.logger.debug("[QueryManager] Remote Query Hydration - Starting", {
      queryHash: query.hash,
      query: query.mainQuery.query,
    });

    const results = await this.databaseService.queryRemote<TableModel<T>[]>(
      query.mainQuery.query,
      query.mainQuery.vars
    );

    this.logger.debug(
      "[QueryManager] Remote Query Hydration - Remote query done",
      {
        queryHash: query.hash,
        resultLength: results?.length ?? 0,
      }
    );

    const hydrateQuery = results
      .map(
        ({ id, ...payload }) =>
          `UPSERT ${id} CONTENT ${this.surrealStringify(
            query.tableName,
            payload
          )}`
      )
      .join(";\n");

    this.logger.debug(
      "[QueryManager] Remote Query Hydration - Updating local cache",
      {
        queryHash: query.hash,
        hydrateQuery,
      }
    );

    await this.databaseService.queryLocal(hydrateQuery);
    this.logger.debug(
      "[QueryManager] Remote Query Hydration - Local cache updated",
      {
        queryHash: query.hash,
      }
    );

    await this.triggerQueryUpdate(query.hash);
  }

  private async hydrateRemoteCreateUpdate<
    T extends { columns: Record<string, ColumnSchema> }
  >(queryHash: number, record: Record<string, unknown>): Promise<void> {
    const query = this.cache[queryHash];
    if (!query) {
      this.logger.error(
        "[QueryManager] Hydrate Remote Create Update - Query not found",
        {
          queryHash: queryHash,
        }
      );
      return;
    }

    this.logger.debug(
      "[QueryManager] Hydrate Remote Create Update - Starting",
      {
        queryHash,
        record,
      }
    );

    const hydrateQuery = `UPSERT ${record.id} CONTENT ${this.surrealStringify(
      query.innerQuery.tableName,
      record
    )}`;

    this.logger.debug(
      "[QueryManager] Remote Query Hydration - Updating local cache",
      {
        queryHash,
        hydrateQuery,
      }
    );

    await this.databaseService.queryLocal(hydrateQuery);
    this.logger.debug(
      "[QueryManager] Remote Query Hydration - Local cache updated",
      {
        queryHash,
      }
    );

    await this.triggerQueryUpdate(queryHash);
  }

  private async hydrateRemoteDelete<
    T extends { columns: Record<string, ColumnSchema> }
  >(queryHash: number, record: Record<string, unknown>): Promise<void> {
    const query = this.cache[queryHash];
    if (!query) {
      this.logger.error(
        "[QueryManager] Hydrate Remote Delete - Query not found",
        {
          queryHash: queryHash,
        }
      );
      return;
    }

    const recordId = record.id as RecordId;

    const hydrateQuery = `DELETE $recordId`;

    this.logger.debug(
      "[QueryManager] Remote Query Hydration - Updating local cache",
      {
        queryHash,
        hydrateQuery,
      }
    );

    await this.databaseService.queryLocal(hydrateQuery, { recordId });
    this.logger.debug(
      "[QueryManager] Remote Query Hydration - Local cache updated",
      {
        queryHash,
      }
    );

    await this.triggerQueryUpdate(queryHash);
  }

  private async handleRemoteUpdate<
    T extends { columns: Record<string, ColumnSchema> }
  >(
    queryHash: number,
    action: "CREATE" | "UPDATE" | "DELETE" | "CLOSE",
    result: Record<string, unknown>
  ): Promise<void> {
    switch (action) {
      case "CREATE":
        this.logger.debug("[QueryManager] Live Event - Created:", result);
        await this.hydrateRemoteCreateUpdate(queryHash, result);
        break;
      case "UPDATE":
        this.logger.debug("[QueryManager] Live Event - Updated:", result);
        await this.hydrateRemoteCreateUpdate(queryHash, result);
        break;
      case "DELETE":
        this.logger.debug("[QueryManager] Live Event - Deleted:", result);
        await this.hydrateRemoteDelete(queryHash, result);
        break;
      default:
        this.logger.error(
          "[QueryManager] Live Event - failed to handle remote update",
          action
        );
    }
  }

  private async subscribeRemoteQuery<
    T extends { columns: Record<string, ColumnSchema> }
  >(query: InnerQuery<T, boolean>): Promise<void> {
    this.logger.debug("[QueryManager] Subscribe Remote Query - Starting", {
      queryHash: query.hash,
      query: query.selectLiveQuery.query,
    });

    const [liveUuid] = await this.databaseService.queryRemote<Uuid[]>(
      query.selectLiveQuery.query,
      query.selectLiveQuery.vars
    );

    this.logger.debug(
      "[QueryManager] Subscribe Remote Query - Created Live UUID",
      {
        queryHash: query.hash,
        liveUuid: liveUuid,
      }
    );

    const handler: LiveHandler<Record<string, unknown>> = async (
      action,
      update
    ) => {
      this.eventSystem.addEvent({
        type: GlobalQueryEventTypes.RemoteLiveUpdate,
        payload: {
          queryHash: query.hash,
          action: action,
          update: update as Record<string, unknown>,
        },
      });
    };

    await this.databaseService.subscribeLiveOfRemote(liveUuid, handler);

    this.logger.debug(
      "[QueryManager] Subscribe Remote Query - Subscribed to Live UUID",
      {
        queryHash: query.hash,
        liveUuid: liveUuid,
      }
    );
  }

  private async initQuery(queryHash: number): Promise<void> {
    const { innerQuery: query, eventSystem: queryEventSystem } =
      this.cache[queryHash];
    try {
      if (!query) {
        this.logger.error("[QueryManager] Update query - Query not found", {
          queryHash: queryHash,
        });
        return;
      }

      this.logger.debug("[QueryManager] Run - Initialize query", {
        queryHash: query.hash,
      });

      this.logger.debug("[QueryManager] Run - Refresh query locally", {
        queryHash: query.hash,
      });

      try {
        await this.queryLocalRefresh(query.hash);
      } catch (error) {
        this.logger.error("Failed to refresh query locally", error);
      }

      this.logger.debug("[QueryManager] Run - Hydrate remote query", {
        queryHash: query.hash,
      });

      try {
        await this.queryRemoteHydration(query);
      } catch (error) {
        this.logger.warn(
          "[QueryManager] Remote hydration failed (continuing with local data)",
          error
        );
        this.authManager.reauthenticate();
      }

      this.logger.debug("[QueryManager] Run - Subscribe to remote query", {
        queryHash: query.hash,
      });

      try {
        await this.subscribeRemoteQuery(query);
      } catch (error) {
        this.logger.warn(
          "[QueryManager] Remote subscription failed (continuing with local data)",
          error
        );
      }

      this.logger.debug(
        "[QueryManager] Run - Initialize subqueries",
        query.hash
      );

      const cleanups: (() => void)[] = [];
      for (const subquery of query.subqueries) {
        await this.run(subquery);

        const sQuery = this.cache[subquery.hash];
        const subId = sQuery.eventSystem.subscribe(
          QueryEventTypes.Updated,
          () =>
            this.eventSystem.addEvent({
              type: GlobalQueryEventTypes.SubqueryUpdated,
              payload: {
                queryHash: query.hash,
                subqueryHash: subquery.hash,
              },
            })
        );

        cleanups.push(() => {
          sQuery.eventSystem.unsubscribe(subId);
        });
      }

      queryEventSystem.subscribe(QueryEventTypes.Destroyed, (e) => {
        cleanups.forEach((cleanup) => cleanup());
      });
    } catch (error) {
      this.logger.error("Failed to initialize query", error);
    }
  }

  run<T extends { columns: Record<string, ColumnSchema> }>(
    query: InnerQuery<T, boolean>
  ): number {
    const cacheHit = this.cache[query.hash];

    if (!cacheHit) {
      this.logger.debug("[QueryManager] Run - Cache miss", {
        queryHash: query.hash,
      });

      this.cache[query.hash] = {
        innerQuery: query,
        eventSystem: createQueryEventSystem(),
      };

      this.eventSystem.addEvent({
        type: GlobalQueryEventTypes.RequestInit,
        payload: {
          queryHash: query.hash,
        },
      });

      return query.hash;
    } else {
      this.logger.debug("[QueryManager] Run - Cache hit", query.hash);
      return query.hash;
    }
  }

  subscribe(
    queryHash: number,
    callback: (data: Record<string, unknown>[]) => void,
    options?: EventSubscriptionOptions
  ): number {
    const query = this.cache[queryHash];
    if (!query) {
      this.logger.error("[QueryManager] Subscribe to Query - Query not found", {
        queryHash: queryHash,
      });
      throw new Error(`Query ${queryHash} not found`);
    }

    return query.eventSystem.subscribe(
      QueryEventTypes.Updated,
      (event) => {
        callback(event.payload.data);
      },
      options
    );
  }

  unsubscribe(queryHash: number, subscriptionId: number): void {
    const query = this.cache[queryHash];
    if (!query) {
      this.logger.error(
        "[QueryManager] Unsubscribe from Query - Query not found",
        {
          queryHash: queryHash,
        }
      );
      throw new Error(`Query ${queryHash} not found`);
    }

    query.eventSystem.unsubscribe(subscriptionId);
  }
}

export function createQueryManagerService<S extends SchemaStructure>(
  schema: S,
  databaseService: DatabaseService,
  authManager: AuthManagerService,
  logger: Logger,
  eventSystem: SpookyEventSystem
): QueryManagerService<S> {
  return new QueryManagerService(
    schema,
    databaseService,
    authManager,
    logger,
    eventSystem
  );
}
