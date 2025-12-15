import { Surreal } from "surrealdb";
import { CacheStrategy, SpookyConfig } from "./config.js";
import { SchemaStructure } from "@spooky/query-builder";
import {
  DatabaseService,
  LocalDatabaseError,
  makeAuthenticateRemoteDatabase,
  makeClearLocalCache,
  makeCloseLocalDatabase,
  makeCloseRemoteDatabase,
  makeDeauthenticateRemoteDatabase,
  makeSubscribeLiveOfRemoteDatabase,
  makeUnsubscribeLiveOfRemoteDatabase,
  makeQueryLocalDatabase,
  makeQueryRemoteDatabase,
  makeUseLocalDatabase,
  makeUseRemoteDatabase,
  RemoteDatabaseError,
} from "./database.js";
import { surrealdbWasmEngines } from "@surrealdb/wasm";
import { Logger } from "./logger.js";

export async function connectRemoteDatabase(
  url: string,
  logger: Logger
): Promise<Surreal> {
  logger.debug(`[DatabaseService] Connecting to remote database...`);

  const startTime = performance.now();

  try {
    const r = new Surreal();
    await r.connect(url, { versionCheck: false });

    const endTime = performance.now();
    const duration = (endTime - startTime).toFixed(2);

    logger.info(`[DatabaseService] ✅ Connected successfully! (${duration}ms)`);
    logger.debug(`[DatabaseService] URL: ${url}`);
    logger.info(
      `[DatabaseService] ✅ Connected successfully to remote database`
    );

    return r;
  } catch (error) {
    throw new RemoteDatabaseError(
      "Failed to connect to remote database",
      error
    );
  }
}

export async function createLocalDatabase(
  dbName: string,
  strategy: CacheStrategy,
  logger: Logger,
  namespace?: string,
  database?: string
): Promise<Surreal> {
  logger.debug(`[DatabaseService] Creating WASM Surreal instance...`);
  logger.debug(`[DatabaseService] Storage strategy: ${strategy}`);
  logger.debug(`[DatabaseService] DB Name: ${dbName}`);

  try {
    const instance = new Surreal({
      engines: surrealdbWasmEngines(),
    });

    // Determine connection URL based on storage strategy
    let connectionUrl: string;
    if (strategy === "indexeddb") {
      // Using indxdb:// protocol for IndexedDB storage
      connectionUrl = `indxdb://${dbName}`;
    } else {
      // Using mem:// protocol for in-memory storage
      connectionUrl = "mem://";
    }

    const startTime = performance.now();
    await instance.connect(connectionUrl);
    const endTime = performance.now();

    const selectedNamespace = namespace || "main";
    const selectedDatabase = database || dbName;
    await instance.use({
      namespace: selectedNamespace,
      database: selectedDatabase,
    });

    const duration = (endTime - startTime).toFixed(2);

    logger.info(`[DatabaseService] ✅ Connected successfully! (${duration}ms)`);
    logger.info(
      `[DatabaseService] ✅ Local database fully initialized! (${selectedNamespace}/${selectedDatabase})`
    );

    return instance;
  } catch (error: any) {
    throw new LocalDatabaseError("Failed to create local database", error);
  }
}

let localDatabase: Surreal | undefined;
let internalDatabase: Surreal | undefined;
let remoteDatabase: Surreal | undefined;

export async function createDatabaseService<S extends SchemaStructure>(
  config: SpookyConfig<S>,
  logger: Logger
): Promise<DatabaseService> {
  const { localDbName, storageStrategy, namespace, database, remoteUrl } =
    config;

  internalDatabase = await createLocalDatabase(
    localDbName,
    storageStrategy,
    logger,
    "internal",
    "main"
  );

  localDatabase = await createLocalDatabase(
    localDbName,
    storageStrategy,
    logger,
    namespace,
    database
  );

  remoteDatabase = await connectRemoteDatabase(remoteUrl, logger);
  if (namespace && database) {
    await remoteDatabase.use({ namespace, database });
  }

  return {
    useLocal: makeUseLocalDatabase(localDatabase),
    useInternal: makeUseLocalDatabase(internalDatabase),
    useRemote: makeUseRemoteDatabase(remoteDatabase),
    queryLocal: makeQueryLocalDatabase(localDatabase, logger),
    queryInternal: makeQueryLocalDatabase(internalDatabase, logger),
    queryRemote: makeQueryRemoteDatabase(remoteDatabase, logger),
    subscribeLiveOfRemote: makeSubscribeLiveOfRemoteDatabase(remoteDatabase),
    unsubscribeLiveOfRemote:
      makeUnsubscribeLiveOfRemoteDatabase(remoteDatabase),
    authenticate: makeAuthenticateRemoteDatabase(remoteDatabase),
    deauthenticate: makeDeauthenticateRemoteDatabase(remoteDatabase),
    closeRemote: makeCloseRemoteDatabase(remoteDatabase),
    closeLocal: makeCloseLocalDatabase(localDatabase),
    closeInternal: makeCloseLocalDatabase(internalDatabase),
    clearLocalCache: makeClearLocalCache(localDatabase),
  };
}
