import { describe, it, expect } from 'vitest';
import type { WhereNode } from '@spooky-sync/query-builder';
import {
  comparisonSql,
  renderWhereSql,
  renderOrderSql,
  scalar,
  serializeRow,
  reviveRow,
  project,
} from './sqlite-plan-sql';

describe('comparisonSql param slaving (aa4af79b)', () => {
  it('prefers params[paramRef] over the baked literal', () => {
    const bind: unknown[] = [];
    const sql = comparisonSql(
      { field: 'id', op: '=', value: 'thread:A', paramRef: 'id' },
      bind,
      { id: 'thread:B' }
    );
    expect(sql).toBe('id = ?');
    expect(bind).toEqual(['thread:B']);
  });

  it('falls back to the baked literal when the param key is absent', () => {
    const bind: unknown[] = [];
    comparisonSql({ field: 'id', op: '=', value: 'thread:A', paramRef: 'id' }, bind, {});
    expect(bind).toEqual(['thread:A']);
  });

  it('a pure $-ref (no baked value) always reads the param', () => {
    const bind: unknown[] = [];
    comparisonSql({ field: 'owner', op: '=', value: undefined, paramRef: 'auth' }, bind, {
      auth: 'user:1',
    });
    expect(bind).toEqual(['user:1']);
  });

  it('renders non-id fields via json_extract and honors swap', () => {
    const bind: unknown[] = [];
    const sql = comparisonSql({ field: 'votes', op: '<', value: 5, swap: true }, bind, {});
    expect(sql).toBe(`? < json_extract(data, '$.votes')`);
    expect(bind).toEqual([5]);
  });
});

describe('renderWhereSql', () => {
  it('joins top-level nodes with AND and parenthesizes OR groups', () => {
    const nodes: WhereNode[] = [
      { field: 'kind', op: '=', value: 'a' },
      {
        or: [
          { field: 'votes', op: '>', value: 1 },
          { field: 'votes', op: '=', value: 0 },
        ],
      },
    ];
    const bind: unknown[] = [];
    const sql = renderWhereSql(nodes, bind, {});
    expect(sql).toBe(
      `json_extract(data, '$.kind') = ? AND (json_extract(data, '$.votes') > ? OR json_extract(data, '$.votes') = ?)`
    );
    expect(bind).toEqual(['a', 1, 0]);
  });
});

describe('renderOrderSql', () => {
  it('renders multi-key ordering', () => {
    expect(renderOrderSql([['a', 'desc'], ['b', 'asc']])).toBe(
      ` ORDER BY json_extract(data, '$.a') DESC, json_extract(data, '$.b') ASC`
    );
  });
});

describe('scalar', () => {
  it('binds RecordId-shaped objects as table:id strings', () => {
    expect(scalar({ tb: 'user', id: '1' })).toBe('user:1');
    expect(scalar('plain')).toBe('plain');
    expect(scalar(3)).toBe(3);
    expect(scalar(null)).toBeNull();
    expect(scalar(undefined)).toBeNull();
  });
});

describe('serializeRow / reviveRow round-trip', () => {
  it('tags Uint8Array as {__u8} and revives it', () => {
    const json = serializeRow({ id: 's:1', blob: new Uint8Array([0, 128, 255]) });
    expect(json).toContain('"__u8"');
    const back = reviveRow(json);
    expect(back.blob).toBeInstanceOf(Uint8Array);
    expect([...(back.blob as Uint8Array)]).toEqual([0, 128, 255]);
  });

  it('serializes RecordId-shaped links to strings and takes the fast parse path otherwise', () => {
    const json = serializeRow({ id: 't:1', author: { tb: 'user', id: 'u1' }, n: 2 });
    expect(reviveRow(json)).toEqual({ id: 't:1', author: 'user:u1', n: 2 });
  });
});

describe('project', () => {
  it('keeps id plus the listed fields only, skipping absent ones', () => {
    expect(project({ id: 'a', x: 1, y: 2 }, ['x', 'missing'])).toEqual({ id: 'a', x: 1 });
  });
});
