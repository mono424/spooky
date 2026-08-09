import { describe, it, expect } from 'vitest';
import { SqliteCacheEngine } from './sqlite-cache-engine';
import { stubTransport } from './sqlite-transport.fixture';
import { translateSurql } from './surql-translate';

/**
 * The statements the DevTools Database explorer emits against the LOCAL store.
 * None of them come from `surql`, so they only ever exercised the SurrealDB
 * engine: on SQLite the paging read threw "unsupported SurrealQL for
 * translation: SELECT * FROM game_insight LIMIT 20 START 0", the table list came
 * back empty (`INFO FOR DB` lowered to a noop) and row edit/delete never
 * matched (they inline a literal record id).
 */

function makeLogger(): any {
  const noop = () => {};
  const l: any = { debug: noop, info: noop, warn: noop, error: noop, trace: noop };
  l.child = () => l;
  return l;
}

/** Records every exec/run SQL so the paging window can be asserted. */
function recordingEngine(rows: Record<string, unknown[]> = {}) {
  const sql: string[] = [];
  const engine = new SqliteCacheEngine({ namespace: 'n', database: 'd' } as any, makeLogger());
  stubTransport(engine, (type, payload: any) => {
    if (type === 'open') return { persisted: true };
    if (type === 'exec' || type === 'run') {
      sql.push(payload.sql);
      for (const [needle, out] of Object.entries(rows)) {
        if (payload.sql.includes(needle)) return { rows: out };
      }
      return { rows: [] };
    }
    return {};
  });
  return { engine, sql };
}

describe('DevTools table paging (LIMIT/START)', () => {
  it('translates the window instead of throwing', () => {
    const { ops } = translateSurql('SELECT * FROM game_insight LIMIT 20 START 0', {});
    expect(ops).toEqual([
      {
        kind: 'selectTable',
        table: 'game_insight',
        where: undefined,
        orderBy: undefined,
        select: undefined,
        value: undefined,
        limit: 20,
        start: 0,
      },
    ]);
  });

  it('renders LIMIT/OFFSET in SQL, page 3 of 20', async () => {
    const { engine, sql } = recordingEngine();
    await engine.connect('anon');
    await engine.query('SELECT * FROM game_insight LIMIT 20 START 40');
    expect(sql.some((s) => /SELECT data FROM "game_insight" LIMIT 20 OFFSET 40$/.test(s))).toBe(
      true
    );
  });

  it('keeps WHERE and ORDER BY alongside the window', () => {
    const { ops } = translateSurql(
      'SELECT * FROM game WHERE done = true ORDER BY date desc LIMIT 10 START 30',
      {}
    );
    expect(ops[0]).toMatchObject({
      kind: 'selectTable',
      table: 'game',
      where: [{ field: 'done', op: '=', value: true }],
      orderBy: [['date', 'desc']],
      limit: 10,
      start: 30,
    });
  });

  it('leaves a LIMIT that is part of a string literal alone', () => {
    const { ops } = translateSurql("SELECT * FROM game WHERE note = 'LIMIT 5'", {});
    expect(ops[0]).toMatchObject({
      kind: 'selectTable',
      where: [{ field: 'note', op: '=', value: 'LIMIT 5' }],
      limit: undefined,
      start: undefined,
    });
  });

  it('a START with no LIMIT still renders valid SQLite', async () => {
    const { engine, sql } = recordingEngine();
    await engine.connect('anon');
    await engine.query('SELECT * FROM game START 5');
    // SQLite has no bare OFFSET — `LIMIT -1` is its "everything" sentinel.
    expect(sql.some((s) => s.endsWith('LIMIT -1 OFFSET 5'))).toBe(true);
  });
});

describe('DevTools row count', () => {
  it("answers `SELECT count() FROM t GROUP ALL` in SurrealDB's shape", async () => {
    const { engine } = recordingEngine({ 'COUNT(*)': [{ n: 137 }] });
    await engine.connect('anon');
    const res = await engine.query<[{ count: number }[]]>(
      'SELECT count() FROM game_insight GROUP ALL'
    );
    expect(res[0]).toEqual([{ count: 137 }]);
  });

  it('counts zero rows as 0, not undefined', async () => {
    const { engine } = recordingEngine();
    await engine.connect('anon');
    const res = await engine.query<[{ count: number }[]]>('SELECT count() FROM empty GROUP ALL');
    expect(res[0]).toEqual([{ count: 0 }]);
  });
});

describe('DevTools table list', () => {
  it('answers INFO FOR DB from sqlite_master, minus SQLite internals', async () => {
    const { engine } = recordingEngine({
      sqlite_master: [{ name: 'game' }, { name: '_00_query' }],
    });
    await engine.connect('anon');
    const [info] = await engine.query<[{ tables: Record<string, string> }]>('INFO FOR DB');
    expect(Object.keys(info.tables)).toEqual(['game', '_00_query']);
  });
});

describe('DevTools row edit / delete (literal record ids)', () => {
  it('UPDATE <table>:<id> MERGE $updates writes that row', async () => {
    const { engine, sql } = recordingEngine();
    await engine.connect('anon');
    await engine.query('UPDATE game:abc MERGE $updates', { updates: { white: 'hikaru' } });
    const write = sql.find((s) => s.startsWith('INSERT INTO "game"'));
    expect(write).toBeDefined();
    expect(write).toContain('json_patch');
  });

  it('DELETE <table>:<id> deletes one row, DELETE <table> still clears the table', () => {
    expect(translateSurql('DELETE game:abc', {}).ops[0]).toEqual({
      kind: 'delete',
      id: 'game:abc',
    });
    expect(translateSurql('DELETE game', {}).ops[0]).toEqual({ kind: 'deleteAll', table: 'game' });
  });

  it('refuses a table-wide MERGE rather than writing a row named after the table', () => {
    // `UPDATE game MERGE $x` means every row of `game` in SurrealQL. Unsupported
    // here — and it must SAY so, not silently create `game:undefined`.
    expect(() => translateSurql('UPDATE game MERGE $x', { x: {} })).toThrow(
      /unsupported SurrealQL/
    );
  });
});
