import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import sqlite3InitModule from '@sqlite.org/sqlite-wasm';
import type { QueryPlan } from '@spooky-sync/query-builder';
import { executeSelect, type SelectDb } from './sqlite-select';
import { serializeRow } from './sqlite-plan-sql';
import type { Row } from './cache-engine';

/**
 * Integration: `executeSelect` (the worker-side plan executor) against a REAL
 * in-memory SQLite (the same @sqlite.org/sqlite-wasm build the worker loads).
 * The scripted-responder tests in `sqlite-select.test.ts` prove parity of the
 * statement sequence; these prove the SQL itself is correct — json_extract
 * filtering, ORDER BY, LIMIT/OFFSET, IN matching, per-parent relation
 * order/limit, and the JSON round-trip of row bodies.
 */

let db: any;
let selectDb: SelectDb;

function seed(table: string, rows: Row[]): void {
  db.exec({ sql: `CREATE TABLE IF NOT EXISTS "${table}" (id TEXT PRIMARY KEY, data TEXT NOT NULL)` });
  selectDb.knownTables.add(table);
  for (const row of rows) {
    db.exec({
      sql: `INSERT INTO "${table}"(id, data) VALUES(?, ?)`,
      bind: [row.id, serializeRow(row)],
    });
  }
}

beforeAll(async () => {
  const sqlite3: any = await sqlite3InitModule();
  db = new sqlite3.oo1.DB(':memory:', 'c');
  selectDb = {
    exec: (sql, bind) =>
      db.exec({ sql, bind, rowMode: 'object', returnValue: 'resultRows' }) as { data: string }[],
    run: (sql, bind) => {
      db.exec({ sql, bind });
    },
    knownTables: new Set<string>(),
  };

  seed('thread', [
    { id: 'thread:A', title: 'alpha', createdAt: 3, author: 'user:1' },
    { id: 'thread:B', title: 'beta', createdAt: 1, author: 'user:2' },
    { id: 'thread:C', title: 'gamma', createdAt: 2, author: 'user:1' },
  ]);
  seed('comment', [
    { id: 'comment:1', thread: 'thread:B', body: 'b-old', votes: 1, author: 'user:1' },
    { id: 'comment:2', thread: 'thread:B', body: 'b-new', votes: 5, author: 'user:2' },
    { id: 'comment:3', thread: 'thread:B', body: 'b-mid', votes: 3, author: 'user:1' },
    { id: 'comment:4', thread: 'thread:A', body: 'a-only', votes: 2, author: 'user:2' },
  ]);
  seed('user', [
    { id: 'user:1', name: 'ada' },
    { id: 'user:2', name: 'grace' },
  ]);
  seed('snapshot', [{ id: 'snapshot:1', blob: new Uint8Array([1, 2, 250]), kind: 'crdt' }]);
});

afterAll(() => {
  db?.close();
});

describe('executeSelect against real SQLite', () => {
  it('filters via json_extract, slaved to params over the baked literal (aa4af79b)', async () => {
    const plan: QueryPlan = {
      table: 'thread',
      where: [{ field: 'id', op: '=', value: 'thread:A', paramRef: 'id' }],
    };
    const withParam = await executeSelect(plan, { id: 'thread:B' }, selectDb);
    expect(withParam.rows.map((r) => r.id)).toEqual(['thread:B']);

    const withoutParam = await executeSelect(plan, {}, selectDb);
    expect(withoutParam.rows.map((r) => r.id)).toEqual(['thread:A']);
  });

  it('filters on a non-id field and binds a RecordId-shaped value as table:id', async () => {
    const plan: QueryPlan = {
      table: 'thread',
      where: [{ field: 'author', op: '=', value: { tb: 'user', id: '1' } }],
      orderBy: [['createdAt', 'asc']],
    };
    const { rows } = await executeSelect(plan, {}, selectDb);
    expect(rows.map((r) => r.id)).toEqual(['thread:C', 'thread:A']);
  });

  it('applies ORDER BY / LIMIT / OFFSET in SQL', async () => {
    const plan: QueryPlan = {
      table: 'thread',
      orderBy: [['createdAt', 'desc']],
      limit: 2,
      offset: 1,
    };
    const { rows } = await executeSelect(plan, {}, selectDb);
    expect(rows.map((r) => r.id)).toEqual(['thread:C', 'thread:B']);
  });

  it('supports OR groups', async () => {
    const plan: QueryPlan = {
      table: 'thread',
      where: [
        {
          or: [
            { field: 'title', op: '=', value: 'alpha' },
            { field: 'title', op: '=', value: 'beta' },
          ],
        },
      ],
      orderBy: [['title', 'asc']],
    };
    const { rows } = await executeSelect(plan, {}, selectDb);
    expect(rows.map((r) => r.id)).toEqual(['thread:A', 'thread:B']);
  });

  it('resolves a nested relation tree with per-parent order/limit, one batch per level', async () => {
    const plan: QueryPlan = {
      table: 'thread',
      where: [{ field: 'id', op: '=', value: 'thread:B' }],
      relations: [
        {
          alias: 'comments',
          table: 'comment',
          cardinality: 'many',
          foreignKeyField: 'thread',
          orderBy: [['votes', 'desc']],
          limit: 2,
          relations: [
            { alias: 'author', table: 'user', cardinality: 'one', foreignKeyField: 'author' },
          ],
        },
      ],
    };
    const { rows, relationFetches } = await executeSelect(plan, {}, selectDb);
    expect(rows).toHaveLength(1);
    const comments = rows[0].comments as Row[];
    // Top-2 by votes, per parent.
    expect(comments.map((c) => c.id)).toEqual(['comment:2', 'comment:3']);
    // Nested `one` relation attached on each child.
    expect(comments.map((c) => (c.author as Row).name)).toEqual(['grace', 'ada']);
    // Level-ordered batching: one fetch for comments + one for users.
    expect(relationFetches).toBe(2);
  });

  it('materializes an ids window preserving ids order, or ORDER BY when given', async () => {
    const windowPlan: QueryPlan = { table: 'comment', ids: ['comment:3', 'comment:1'] };
    const inIdsOrder = await executeSelect(windowPlan, {}, selectDb);
    expect(inIdsOrder.rows.map((r) => r.id)).toEqual(['comment:3', 'comment:1']);

    const ordered = await executeSelect(
      { ...windowPlan, orderBy: [['votes', 'asc']] },
      {},
      selectDb
    );
    expect(ordered.rows.map((r) => r.id)).toEqual(['comment:1', 'comment:3']);
  });

  it('trims to the projection while keeping id', async () => {
    const plan: QueryPlan = {
      table: 'thread',
      where: [{ field: 'id', op: '=', value: 'thread:A' }],
      select: ['title'],
    };
    const { rows } = await executeSelect(plan, {}, selectDb);
    expect(rows).toEqual([{ id: 'thread:A', title: 'alpha' }]);
  });

  it('revives Uint8Array bodies through the {__u8} tag round-trip', async () => {
    const plan: QueryPlan = {
      table: 'snapshot',
      where: [{ field: 'kind', op: '=', value: 'crdt' }],
    };
    const { rows } = await executeSelect(plan, {}, selectDb);
    expect(rows).toHaveLength(1);
    expect(rows[0].blob).toBeInstanceOf(Uint8Array);
    expect([...(rows[0].blob as Uint8Array)]).toEqual([1, 2, 250]);
  });

  it('creates a missing table on first touch instead of failing', async () => {
    const plan: QueryPlan = { table: 'never_written', where: [{ field: 'x', op: '=', value: 1 }] };
    const { rows } = await executeSelect(plan, {}, selectDb);
    expect(rows).toEqual([]);
    expect(selectDb.knownTables.has('never_written')).toBe(true);
  });
});
