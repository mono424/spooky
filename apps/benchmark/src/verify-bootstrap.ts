#!/usr/bin/env node
/**
 * Verify that the SSP genuinely bootstrapped a large DB into its in-memory
 * circuit store. Six checks, in order:
 *
 *   1. Seed N comments at chosen row size, computing expected total bytes.
 *   2. Start scheduler + SSP. Wait for ready.
 *   3. Register a canary view (SELECT id FROM comment).
 *   4. Read /debug/view/:id, assert cache_size == seeded comment count.
 *   5. Read the same /debug/view/:id, sample cache entries, assert they are
 *      real comment IDs from the seeded range (no synthetic / placeholder
 *      keys, no duplicates).
 *   6. Insert one new comment with a unique marker via POST /ingest, then poll
 *      /debug/view/:id until cache_size grows to N+1 and the new ID appears.
 *
 * Each check prints a pass/fail line and the underlying number so the run
 * is auditable.
 */
import { randomUUID } from "node:crypto";
import { startSurrealOnly } from "./stack/orchestrator.js";
import { seed } from "./workload/seed.js";
import { SchedulerClient } from "./drivers/scheduler-http.js";
import { SspClient } from "./drivers/ssp-http.js";
import { sleep } from "./util/wait.js";
import { log } from "./util/log.js";

interface Args {
  rows: number;
  rowBytes: number;
}

function parseArgs(): Args {
  const argv = process.argv.slice(2);
  const get = (k: string, d?: string) => {
    const i = argv.indexOf(k);
    if (i === -1) return d;
    return argv[i + 1];
  };
  const rows = parseInt(get("--rows", "1000")!, 10);
  const rowBytes = parseInt(get("--row-bytes", "0")!, 10);
  return { rows, rowBytes };
}

function pass(label: string, detail: string): void {
  log.info(`PASS ${label}: ${detail}`);
}

function fail(label: string, detail: string): never {
  log.error(`FAIL ${label}: ${detail}`);
  process.exit(1);
}

async function main() {
  const { rows, rowBytes } = parseArgs();
  log.step(`verify-bootstrap rows=${rows} rowBytes=${rowBytes}`);

  // Step 1: seed.
  const pre = await startSurrealOnly({});
  let seedCounts;
  let stack;
  try {
    seedCounts = await seed(pre.surreal, rows, { commentPadBytes: rowBytes });
    pass(
      "seeded",
      `${seedCounts.comments} comments, ${(seedCounts.approxBytes / 1024 / 1024).toFixed(1)} MiB total`,
    );

    // Step 2: bring up scheduler + SSP.
    stack = await pre.startBackend();
    pass("stack ready", `scheduler=${stack.scheduler.baseUrl} ssp=${stack.ssp.baseUrl}`);
  } catch (e) {
    await pre.stop().catch(() => {});
    throw e;
  }

  try {
    const client = new SchedulerClient(stack.scheduler.baseUrl);
    const ssp = new SspClient(stack.ssp.baseUrl, stack.authSecret);

    // Step 3: register canary.
    const canaryId = `verify:canary:${randomUUID()}`;
    await client.registerView({
      id: canaryId,
      surql: `SELECT id FROM comment`,
      clientId: `verify-bootstrap`,
      ttl: "1h",
      lastActiveAt: new Date().toISOString(),
    });
    pass("registered canary", `id=${canaryId}, query="SELECT id FROM comment"`);

    // Step 4: cache_size must match seeded count.
    const v1 = await ssp.getDebugView(canaryId);
    if (!v1) fail("debug view fetch", "view not found on SSP");
    if (v1.cache_size !== seedCounts.comments) {
      fail(
        "cache_size matches seed",
        `expected ${seedCounts.comments}, got ${v1.cache_size}`,
      );
    }
    pass(
      "cache_size matches seed",
      `cache_size=${v1.cache_size} (= seeded comment count)`,
    );

    // Step 5: cache entries are real comment IDs from the seeded range.
    // The SSP prefixes its cache key with `<table>:` and stores SurrealDB's
    // own record-id form (which may be backtick-quoted). The shapes we
    // accept are `comment:<n>` or `comment:\`comment:<n>\``.
    const idRe = /^comment:(?:`?comment:)?(\d+)`?$/;
    const seededRange = seedCounts.comments;
    const sample = v1.cache.slice(0, 10);
    const extracted = sample.map((c) => {
      const m = idRe.exec(c.key);
      return m ? Number.parseInt(m[1]!, 10) : NaN;
    });
    if (extracted.some((n) => !Number.isFinite(n))) {
      fail(
        "cache holds real comment IDs",
        `did not match comment:N pattern; got: ${JSON.stringify(sample)}`,
      );
    }
    if (extracted.some((n) => n < 0 || n >= seededRange)) {
      fail(
        "cache IDs in seeded range",
        `expected 0..${seededRange - 1}, got ${JSON.stringify(extracted)}`,
      );
    }
    const dupes = new Set(v1.cache.map((c) => c.key)).size !== v1.cache.length;
    if (dupes) fail("cache has unique keys", "duplicate keys in cache");
    pass(
      "cache holds real comment IDs",
      `first 10 IDs in seeded range [0..${seededRange - 1}]: ${extracted.join(", ")}`,
    );

    // Step 6: ingest a unique record, confirm SSP picks it up.
    const marker = `verify-${randomUUID()}`;
    const newId = `comment:verify_${Date.now()}`;
    await client.ingest({
      table: "comment",
      op: "CREATE",
      id: newId,
      record: {
        id: newId,
        thread: "thread:0",
        content: marker,
        author: "user:0",
        created_at: new Date().toISOString(),
      },
    });
    log.info(`ingested ${newId} with marker ${marker.slice(0, 12)}…`);

    const expected = seedCounts.comments + 1;
    const deadline = Date.now() + 15_000;
    let observedSize = -1;
    let foundKey = false;
    // Keys in cache may be wrapped in backticks; check inner record id.
    const innerIdRe = /^comment:(?:`?comment:)?(.+?)`?$/;
    const newInnerId = newId.replace(/^comment:/, "");
    while (Date.now() < deadline) {
      const v = await ssp.getDebugView(canaryId).catch(() => null);
      if (v) {
        observedSize = v.cache_size;
        foundKey = v.cache.some((c) => {
          const m = innerIdRe.exec(c.key);
          return m && m[1] === newInnerId;
        });
        if (observedSize === expected && foundKey) break;
      }
      await sleep(50);
    }
    if (observedSize !== expected) {
      fail(
        "post-ingest cache size",
        `expected ${expected}, got ${observedSize} after 15s`,
      );
    }
    if (!foundKey) {
      fail("post-ingest contains new key", `key ${newId} not present in cache`);
    }
    pass(
      "post-ingest reflects change",
      `cache_size=${observedSize}, includes ${newId}`,
    );

    log.info("");
    log.info("ALL CHECKS PASSED");
  } finally {
    await stack.stop().catch(() => {});
  }
}

main().catch((e) => {
  log.error(e instanceof Error ? e.stack ?? e.message : String(e));
  process.exit(1);
});
