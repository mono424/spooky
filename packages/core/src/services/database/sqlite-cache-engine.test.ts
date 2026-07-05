import { describe, it, expect } from 'vitest';
import { RecordId } from 'surrealdb';
import { pureWriteOpResult } from './sqlite-cache-engine';
import { translateSurql } from './surql-translate';
import type { SqlOp } from './surql-translate';
import { surql } from '../../utils/surql';

// `pureWriteOpResult` is the single source of truth for what a pure-write op
// contributes to a query's per-statement results. The batched fast path in
// `query()` and the per-op `execOp` path BOTH route through it, so a caller that
// reads a statement's output (e.g. `create()` reads `resultIndex:0` for the new
// row + its id) sees the same shape either way.
describe('pureWriteOpResult', () => {
  it('echoes the written row (with id) for an upsert — no read-back', () => {
    const op: SqlOp = {
      kind: 'upsert',
      id: 'connection:CONN_abc',
      data: { provider: 'chesscom', username: 'hikaru' },
      mode: 'replace',
    };
    expect(pureWriteOpResult(op)).toEqual({
      provider: 'chesscom',
      username: 'hikaru',
      id: 'connection:CONN_abc',
    });
  });

  it('stringifies a RecordId id via stableKey', () => {
    const op: SqlOp = {
      kind: 'upsert',
      id: new RecordId('connection', 'CONN_abc'),
      data: { provider: 'lichess' },
      mode: 'replace',
    };
    expect(pureWriteOpResult(op)).toEqual({ provider: 'lichess', id: 'connection:CONN_abc' });
  });

  it('yields [] for delete / deleteAll and null for noop', () => {
    expect(pureWriteOpResult({ kind: 'delete', id: 'game:1' })).toEqual([]);
    expect(pureWriteOpResult({ kind: 'deleteAll', table: 'game' })).toEqual([]);
    expect(pureWriteOpResult({ kind: 'noop' })).toBeNull();
  });
});

// Regression: a single `create()` compiles to an all-upsert transaction
// (createSet for the row + createMutation for the pending-mutation log) and
// extracts `resultIndex:0` for the created row. The SQLite fast path must return
// that row (with its id) at that index — returning `[]` there dropped the id and
// crashed the reconcile in `encodeRecordId` ("reading 'table'").
describe('create() tx result shaping (fast path parity)', () => {
  it('resultIndex:0 carries the created row with its id', () => {
    const rid = new RecordId('connection', 'CONN_abc');
    const mid = new RecordId('_00_pending_mutations', '1');
    const vars = {
      id: rid,
      mid,
      data_provider: 'chesscom',
      data_username: 'hikaru',
    };

    // Same statement pair DataModule.create emits.
    const sealed = surql.seal(
      surql.tx([
        surql.createSet('id', [
          { key: 'provider', variable: 'data_provider' },
          { key: 'username', variable: 'data_username' },
        ]),
        surql.createMutation('create', 'mid', 'id', 'data'),
      ]),
      { resultIndex: 0 }
    );

    const { transaction, ops } = translateSurql(sealed.sql, vars);
    expect(transaction).toBe(true);
    // Both statements are upserts → the engine takes the all-write fast path.
    expect(ops.every((o) => o.kind === 'upsert')).toBe(true);

    // Fast-path shaping: [null (BEGIN), ...one result per statement].
    const shaped = [null, ...ops.map(pureWriteOpResult)];
    const created = sealed.extract(shaped) as unknown as { id: unknown; provider: string };

    expect(created.id).toBe('connection:CONN_abc');
    expect(created.provider).toBe('chesscom');
  });
});
