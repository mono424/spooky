import { QueryManager } from './services/query/query.js';
import { MutationManager } from './services/mutation/mutation.js';
import { SpookyConfig, QueryTimeToLive, SpookyQueryResultPromise } from './types.js';
import {
  LocalDatabaseService,
  LocalMigrator,
  RemoteDatabaseService,
} from './services/database/index.js';
import { Surreal } from 'surrealdb';
import { SpookySync } from './services/sync/index.js';
import {
  GetTable,
  QueryBuilder,
  QueryOptions,
  SchemaStructure,
  TableModel,
  TableNames,
} from '@spooky/query-builder';

export class SpookyClient<S extends SchemaStructure> {
  private local: LocalDatabaseService;
  private remote: RemoteDatabaseService;
  private migrator: LocalMigrator;
  private queryManager: QueryManager;
  private mutationManager: MutationManager;
  private sync: SpookySync;

  constructor(private config: SpookyConfig<S>) {
    if (!this.config.clientId) {
      this.config.clientId = crypto.randomUUID();
    }
    this.local = new LocalDatabaseService(this.config.database);
    this.remote = new RemoteDatabaseService(this.config.database);
    this.migrator = new LocalMigrator(this.local);
    this.mutationManager = new MutationManager(this.local);
    this.queryManager = new QueryManager(this.local, this.remote, this.config.clientId);
    this.sync = new SpookySync(
      this.local,
      this.remote,
      this.mutationManager.events,
      this.queryManager.eventsSystem
    );
  }

  async init() {
    await this.local.connect();
    await this.remote.connect();
    await this.queryManager.init();
    await this.migrator.provision(this.config.schemaSurql);
  }

  async close() {
    await this.local.close();
    await this.remote.close();
  }

  authenticate(token: string) {
    return this.remote.getClient().authenticate(token);
  }

  deauthenticate() {
    return this.remote.getClient().invalidate();
  }

  query<Table extends TableNames<S>>(
    table: Table,
    options: QueryOptions<TableModel<GetTable<S, Table>>, false>,
    ttl: QueryTimeToLive = '10m'
  ): QueryBuilder<S, Table, SpookyQueryResultPromise> {
    return new QueryBuilder<S, Table, SpookyQueryResultPromise>(
      this.config.schema,
      table,
      async (q) => ({
        hash: await this.queryManager.query(q.selectQuery.query, q.selectQuery.vars ?? {}, ttl),
      }),
      options
    );
  }

  async queryRaw(sql: string, params: Record<string, any>, ttl: QueryTimeToLive) {
    return this.queryManager.query(sql, params, ttl);
  }

  async subscribe(
    queryHash: string,
    callback: (records: Record<string, any>[]) => void,
    options?: { immediate?: boolean }
  ): Promise<() => void> {
    return this.queryManager.subscribe(queryHash, callback, options);
  }

  create(table: string, data: Record<string, unknown>) {
    return this.mutationManager.create(table, data);
  }

  update(table: string, id: string, data: Record<string, unknown>) {
    return this.mutationManager.update(table, id, data);
  }

  delete(table: string, id: string) {
    return this.mutationManager.delete(table, id);
  }

  async useRemote<T>(fn: (client: Surreal) => Promise<T> | T): Promise<T> {
    return fn(this.remote.getClient());
  }
}
