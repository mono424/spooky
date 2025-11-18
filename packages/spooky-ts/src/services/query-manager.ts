import {
  ColumnSchema,
  InnerQuery,
  SchemaStructure,
  TableModel,
} from "@spooky/query-builder";
import { AuthManagerService, DatabaseService } from "./index.js";
import { LiveHandler, Uuid } from "surrealdb";
import { decodeFromSpooky } from "./converter.js";
import { Logger } from "./logger.js";

export type CleanupFn = () => void;

export interface Query<
  T extends { columns: Record<string, ColumnSchema> },
  IsOne extends boolean
> {
  innerQuery: InnerQuery<T, IsOne>;
  cleanup: CleanupFn;
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
    private logger: Logger
  ) {}

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

    // First pass: replace Date objects with placeholders
    const replacer = (key: string, val: unknown): unknown => {
      if (table.columns[key]?.dateTime) {
        return DATE_PLACEHOLDER + val + DATE_PLACEHOLDER;
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

    return jsonString;
  }

  private async queryLocalRefresh<
    T extends { columns: Record<string, ColumnSchema> }
  >(query: InnerQuery<T, boolean>): Promise<void> {
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

    const decodedResults = results.map((result) =>
      decodeFromSpooky(this.schema, query.tableName, result)
    );

    query.setData(decodedResults.filter((result) => result !== undefined));
  }

  async refreshTableQueries(table: string): Promise<void> {
    this.logger.debug("[QueryManager] Refresh Table Queries - Starting", {
      table,
    });

    for (const query of Object.values(this.cache)) {
      if (query.innerQuery.tableName === table) {
        try {
          await this.queryLocalRefresh(query.innerQuery);
        } catch (error) {
          this.logger.error("Failed to refresh query", error);
        }
      }
    }

    this.logger.debug("[QueryManager] Refresh Table Queries - Done", {
      table,
    });
  }

  private async queryRemoteHydration<
    T extends { columns: Record<string, ColumnSchema> }
  >(query: InnerQuery<T, boolean>): Promise<void> {
    this.logger.debug("[QueryManager] Remote Query Hydration - Starting", {
      queryHash: query.hash,
      query: query.selectQuery.query,
    });

    const results = await this.databaseService.queryRemote<TableModel<T>[]>(
      query.selectQuery.query,
      query.selectQuery.vars
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
        resultLength: results?.length ?? 0,
      }
    );

    const decodedResults = results.map((result) =>
      decodeFromSpooky(this.schema, query.tableName, result)
    );

    query.setData(decodedResults.filter((result) => result !== undefined));
  }

  private async handleRemoteUpdate<
    T extends { columns: Record<string, ColumnSchema> }
  >(
    query: InnerQuery<T, boolean>,
    action: "CREATE" | "UPDATE" | "DELETE" | "CLOSE",
    result: unknown
  ): Promise<void> {
    switch (action) {
      case "CREATE":
        this.logger.debug("[QueryManager] Live Event - Created:", result);
        break;
      case "UPDATE":
        this.logger.debug("[QueryManager] Live Event - Updated:", result);
        break;
      case "DELETE":
        this.logger.debug("[QueryManager] Live Event - Deleted:", result);
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
      result
    ) => {
      try {
        await this.handleRemoteUpdate(query, action, result);
      } catch (error) {
        this.logger.error("Failed to refresh query after subscription", error);
      }
    };

    await this.databaseService.unsubscribeLiveOfRemote(liveUuid, handler);

    this.logger.debug(
      "[QueryManager] Subscribe Remote Query - Subscribed to Live UUID",
      {
        queryHash: query.hash,
        liveUuid: liveUuid,
      }
    );
  }

  run<T extends { columns: Record<string, ColumnSchema> }>(
    query: InnerQuery<T, boolean>
  ): { cachedQuery: InnerQuery<T, boolean> | null; cleanup: CleanupFn } {
    this.logger.debug("[QueryManager] Run - Starting", {
      queryHash: query.hash,
    });

    const cacheHit = this.cache[query.hash];

    if (!cacheHit) {
      this.logger.debug("[QueryManager] Run - Cache miss", {
        queryHash: query.hash,
      });

      this.cache[query.hash] = {
        innerQuery: query,
        cleanup: () => {},
      };

      // Run initialization asynchronously
      (async () => {
        try {
          this.logger.debug("[QueryManager] Run - Initialize query", {
            queryHash: query.hash,
          });

          this.logger.debug("[QueryManager] Run - Refresh query locally", {
            queryHash: query.hash,
          });

          try {
            await this.queryLocalRefresh(query);
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

          for (const subquery of query.subqueries) {
            const unsubscribe = subquery.subscribe(async () => {
              try {
                await this.queryLocalRefresh(query);
              } catch (error) {
                this.logger.error(
                  "Failed to refresh query after subscription",
                  error
                );
              }
            });

            const previousCleanup = this.cache[query.hash].cleanup;
            this.cache[query.hash].cleanup = () => {
              previousCleanup();
              unsubscribe();
            };

            await this.run(subquery);
          }
        } catch (error) {
          this.logger.error("Failed to initialize query", error);
        }
      })();
      return {
        cachedQuery: null,
        cleanup: this.cache[query.hash].cleanup,
      };
    } else {
      this.logger.debug("[QueryManager] Run - Cache hit", query.hash);
      return {
        cachedQuery: cacheHit.innerQuery,
        cleanup: cacheHit.cleanup,
      };
    }
  }
}

export function createQueryManagerService<S extends SchemaStructure>(
  schema: S,
  databaseService: DatabaseService,
  authManager: AuthManagerService,
  logger: Logger
): QueryManagerService<S> {
  return new QueryManagerService(schema, databaseService, authManager, logger);
}
