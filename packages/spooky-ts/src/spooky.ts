// spooky.ts
import {
  GetTable,
  QueryBuilder,
  QueryOptions,
  SchemaStructure,
  TableModel,
  TableNames,
} from "@spooky/query-builder";
import { RecordId } from "surrealdb";
import { DatabaseService, EventSystem } from "./services/index.js";
import { AuthManagerService } from "./services/auth-manager.js";
import { QueryManagerService } from "./services/query-manager.js";
import { MutationManagerService } from "./services/mutation-manager.js";

export interface SpookyInstance<S extends SchemaStructure> {
  subscribe: typeof EventSystem.prototype.subscribe;
  unsubscribe: typeof EventSystem.prototype.unsubscribe;
  authenticate: (token: string) => Promise<void>;
  deauthenticate: () => Promise<void>;
  create: <N extends TableNames<S>>(
    tableName: N,
    payload: TableModel<GetTable<S, N>>
  ) => Promise<void>;
  update: <N extends TableNames<S>>(
    tableName: N,
    recordId: RecordId,
    payload: Partial<TableModel<GetTable<S, N>>>
  ) => Promise<void>;
  delete: <N extends TableNames<S>>(
    tableName: N,
    id: RecordId
  ) => Promise<void>;
  query: <Table extends TableNames<S>>(
    table: Table,
    options: QueryOptions<TableModel<GetTable<S, Table>>, false>
  ) => QueryBuilder<S, Table>;
  close: () => Promise<void>;
  clearLocalCache: () => Promise<void>;
  useRemote: <T>(fn: (db: any) => T | Promise<T>) => Promise<T>;
}

export async function createSpookyInstance<S extends SchemaStructure>(
  schema: S,
  databaseService: DatabaseService,
  authManager: AuthManagerService,
  queryManager: QueryManagerService<S>,
  mutationManager: MutationManagerService<S>,
  eventSystem: EventSystem
): Promise<SpookyInstance<S>> {
  const useQuery = <Table extends TableNames<S>>(
    table: Table,
    options: QueryOptions<TableModel<GetTable<S, Table>>, false>
  ): QueryBuilder<S, Table> => {
    return new QueryBuilder<S, Table>(
      schema,
      table,
      (q) => queryManager.run(q),
      options
    );
  };

  const close = async (): Promise<void> => {
    await databaseService.closeRemote();
    await databaseService.closeLocal();
    await databaseService.closeInternal();
  };

  return {
    subscribe: eventSystem.subscribe.bind(eventSystem),
    unsubscribe: eventSystem.unsubscribe.bind(eventSystem),
    authenticate: authManager.authenticate.bind(authManager),
    deauthenticate: authManager.deauthenticate.bind(authManager),
    create: <N extends TableNames<S>>(
      tableName: N,
      payload: TableModel<GetTable<S, N>>
    ) => mutationManager.create(tableName, payload),
    update: <N extends TableNames<S>>(
      tableName: N,
      recordId: RecordId,
      payload: Partial<TableModel<GetTable<S, N>>>
    ) => mutationManager.update(tableName, recordId, payload),
    delete: <N extends TableNames<S>>(tableName: N, id: RecordId) =>
      mutationManager.delete(tableName, id),
    query: useQuery,
    close,
    clearLocalCache: databaseService.clearLocalCache,
    useRemote: databaseService.useRemote,
  };
}
