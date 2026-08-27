/**
 * `ingest_many` — bulk ingest as ONE circuit step.
 *
 * The contract that matters: a batch must leave the circuit in exactly the
 * state the same changes applied one-by-one would, because the client picks
 * between the two paths purely on batch size.
 */

import { describe, it, expect, beforeAll } from 'vitest';
import { readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { initSync } from '../pkg/ssp_wasm.js';
import type { WasmViewUpdate } from '../pkg/ssp_wasm';
import { makeProcessor, createViewConfig, makeUserRecord } from './helpers';

const __dirname = dirname(fileURLToPath(import.meta.url));

beforeAll(() => {
  initSync({ module: readFileSync(join(__dirname, '../pkg/ssp_wasm_bg.wasm')) });
});

const VIEW_SQL = 'SELECT * FROM user';

// The view's materialized [id, version] pairs, id-sorted so two processors that
// took different paths to the same state compare equal.
const rowsOf = (update: WasmViewUpdate | undefined) =>
  [...((update?.result_data ?? []) as [string, number][])].sort((a, b) =>
    a[0].localeCompare(b[0])
  );

describe('ingest_many', () => {
  it('leaves the same view state as the same changes ingested one by one', () => {
    const users = Array.from({ length: 25 }, (_, i) =>
      makeUserRecord(`user${i}`, `user${i}@example.com`)
    );

    const single = makeProcessor();
    single.register_view(createViewConfig('v', VIEW_SQL));
    for (const u of users) single.ingest('user', 'CREATE', u.id, u.record);

    const bulk = makeProcessor();
    bulk.register_view(createViewConfig('v', VIEW_SQL));
    const updates = bulk.ingest_many(
      users.map((u) => ({ table: 'user', op: 'CREATE', id: u.id, record: u.record }))
    ) as WasmViewUpdate[];

    // One step, so one update for the one affected view — not 25.
    expect(updates).toHaveLength(1);
    expect(updates[0].query_id).toBe('v');
    expect(rowsOf(updates[0])).toHaveLength(25);

    const singleFinal = single.register_view(createViewConfig('check', VIEW_SQL));
    const bulkFinal = bulk.register_view(createViewConfig('check', VIEW_SQL));
    expect(rowsOf(bulkFinal)).toEqual(rowsOf(singleFinal));
    expect(bulkFinal?.result_hash).toBe(singleFinal?.result_hash);
  });

  it('applies creates, updates and deletes within one batch in order', () => {
    const [a, b, c] = [
      makeUserRecord('a', 'a@example.com'),
      makeUserRecord('b', 'b@example.com'),
      makeUserRecord('c', 'c@example.com'),
    ];

    const mixed = [
      { table: 'user', op: 'CREATE', id: a.id, record: a.record },
      { table: 'user', op: 'CREATE', id: b.id, record: b.record },
      { table: 'user', op: 'CREATE', id: c.id, record: c.record },
      // Same id twice in one batch: the later change wins, as it would if the
      // two arrived as separate ingests.
      { table: 'user', op: 'UPDATE', id: b.id, record: { ...b.record, username: 'b2' } },
      { table: 'user', op: 'DELETE', id: c.id, record: c.record },
    ];

    const single = makeProcessor();
    single.register_view(createViewConfig('v', VIEW_SQL));
    for (const m of mixed) single.ingest(m.table, m.op, m.id, m.record);

    const bulk = makeProcessor();
    bulk.register_view(createViewConfig('v', VIEW_SQL));
    bulk.ingest_many(mixed);

    const singleFinal = single.register_view(createViewConfig('check', VIEW_SQL));
    const bulkFinal = bulk.register_view(createViewConfig('check', VIEW_SQL));
    expect(rowsOf(bulkFinal).map(([id]) => id)).not.toContain(c.id);
    expect(rowsOf(bulkFinal)).toEqual(rowsOf(singleFinal));
    expect(bulkFinal?.result_hash).toBe(singleFinal?.result_hash);
  });

  it('is a no-op on an empty batch', () => {
    const processor = makeProcessor();
    processor.register_view(createViewConfig('v', VIEW_SQL));
    expect(processor.ingest_many([])).toEqual([]);
  });

  it('reports only the views a batch actually touched', () => {
    const processor = makeProcessor();
    processor.register_view(createViewConfig('users', VIEW_SQL));
    processor.register_view(createViewConfig('products', 'SELECT * FROM product'));

    const u = makeUserRecord('solo', 'solo@example.com');
    const updates = processor.ingest_many([
      { table: 'user', op: 'CREATE', id: u.id, record: u.record },
    ]) as WasmViewUpdate[];

    expect(updates.map((x) => x.query_id)).toEqual(['users']);
  });
});
