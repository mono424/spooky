import path from "node:path";
import fs from "node:fs";
import { GenericContainer, type StartedTestContainer, Wait } from "testcontainers";
import { Surreal } from "surrealdb";
import { log } from "../util/log.js";

export interface SurrealHandle {
  /** ws://host:port */
  wsUrl: string;
  /** http://host:port */
  httpUrl: string;
  namespace: string;
  database: string;
  username: string;
  password: string;
  stop: () => Promise<void>;
}

export interface SurrealOptions {
  image?: string;
  modulesDir?: string;
  schemaPath?: string;
  namespace?: string;
  database?: string;
  startupTimeoutMs?: number;
  /** When true, pipe SurrealDB stdout to our stdout (noisy). */
  verbose?: boolean;
}

const DEFAULT_IMAGE = "surrealdb/surrealdb:v3.0.0";

function defaultModulesDir(): string {
  // Compiled location: apps/benchmark/dist/stack/surrealdb.js → 4 up = repo root.
  return path.resolve(import.meta.dirname, "../../../..", "tests/.spooky");
}

function defaultSchemaPath(modulesDir: string): string {
  // Prefer the simple user-authored schema (tables only). The generated
  // schema.gen.surql includes DEFINE EVENT triggers that http::post to
  // hostnames like `ssp` only resolvable inside a Docker network, which
  // breaks seeding. The benchmark drives /ingest directly via the scheduler
  // HTTP API anyway, so we don't need those auto-ingest events.
  const simple = path.resolve(modulesDir, "..", "schema.surql");
  return simple;
}

export async function startSurrealDB(opts: SurrealOptions = {}): Promise<SurrealHandle> {
  const image = opts.image ?? DEFAULT_IMAGE;
  const modulesDir = opts.modulesDir ?? defaultModulesDir();
  const schemaPath = opts.schemaPath ?? defaultSchemaPath(modulesDir);
  const namespace = opts.namespace ?? "bench_ns";
  const database = opts.database ?? "bench_db";
  const startupTimeoutMs = opts.startupTimeoutMs ?? 60_000;

  if (!fs.existsSync(modulesDir)) {
    throw new Error(
      `SurrealDB modules dir not found: ${modulesDir}. Run \`pnpm -F @spooky-sync/tests run generate\` first.`,
    );
  }

  log.info(`Starting SurrealDB container (${image})`);

  const xorPath = path.join(modulesDir, "xor_module.surli");
  const sspPath = path.join(modulesDir, "ssp_module.surli");
  const bindMounts: { source: string; target: string }[] = [];
  if (fs.existsSync(xorPath))
    bindMounts.push({ source: xorPath, target: "/modules/xor_module.surli" });
  if (fs.existsSync(sspPath))
    bindMounts.push({ source: sspPath, target: "/modules/ssp_module.surli" });

  let container: StartedTestContainer = await new GenericContainer(image)
    .withExposedPorts(8000)
    .withEnvironment({
      SURREAL_BUCKET_FOLDER_ALLOWLIST: "/modules",
      SURREAL_CAPS_ALLOW_EXPERIMENTAL: "surrealism,files,kv",
    })
    .withBindMounts(bindMounts)
    .withUser("root")
    .withCommand([
      "start",
      "--log",
      "info",
      "--user",
      "root",
      "--pass",
      "root",
      "--allow-all",
      "--allow-experimental",
    ])
    .withStartupTimeout(startupTimeoutMs)
    .withWaitStrategy(Wait.forLogMessage("Started web server on 0.0.0.0:8000"))
    .start();

  const port = container.getMappedPort(8000);
  const rawHost = container.getHost();
  // testcontainers may return "localhost", which resolves to ::1 on macOS and
  // some Docker setups don't bind the IPv6 loopback. Force IPv4 to avoid an
  // intermittent "Connection refused" when child processes connect.
  const host = rawHost === "localhost" ? "127.0.0.1" : rawHost;
  const httpUrl = `http://${host}:${port}`;
  const wsUrl = `ws://${host}:${port}`;
  log.info(`SurrealDB listening at ${httpUrl}`);

  const db = new Surreal();
  await db.connect(`${httpUrl}/rpc`);
  await db.signin({ username: "root", password: "root" });
  await db.use({ namespace, database });

  if (fs.existsSync(schemaPath)) {
    let schema = fs.readFileSync(schemaPath, "utf8");
    schema = schema.replace("file:/modules", "file:///modules");
    log.info(`Applying schema from ${schemaPath}`);
    const res = (await db.query(schema)) as unknown;
    if (Array.isArray(res)) {
      for (const r of res as Array<{ status?: string; result?: unknown }>) {
        if (r && r.status === "ERR") {
          throw new Error(`Schema apply failed: ${JSON.stringify(r)}`);
        }
      }
    }
  } else {
    log.warn(`Schema file not found at ${schemaPath}, proceeding without applying it.`);
  }

  await db.close();

  return {
    wsUrl,
    httpUrl,
    namespace,
    database,
    username: "root",
    password: "root",
    stop: async () => {
      log.info("Stopping SurrealDB container");
      await container.stop({ timeout: 10_000 });
    },
  };
}

/** Connect a fresh client to the running SurrealDB. */
export async function connectSurreal(handle: SurrealHandle): Promise<Surreal> {
  const db = new Surreal();
  await db.connect(`${handle.httpUrl}/rpc`);
  await db.signin({ username: handle.username, password: handle.password });
  await db.use({ namespace: handle.namespace, database: handle.database });
  return db;
}
