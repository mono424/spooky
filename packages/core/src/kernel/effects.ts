import type { QueryPlan } from '@spooky-sync/query-builder';
import type { SealedQuery } from '../utils/surql';
import type { ClientState } from '../state/client-state';
import type { IngestRecord } from '../services/stream-processor/index';
import type { RecordVersionArray, RegistrationTimings } from '../types';
import type { OutEvent, RuntimeEvent } from './events';

export type Vars = Record<string, unknown>;

/** What the in-browser SSP needs to build a local view. */
export interface RegisterPlan {
  queryHash: string;
  surql: string;
  params: Vars;
  ttl: string;
  tableName: string;
  involvedTables?: string[];
}
export interface RegisterResult {
  localArray: RecordVersionArray;
  timings: RegistrationTimings;
}

/** One statement's outcome of a multi-statement remote request. */
export type StatementResult =
  | { status: 'OK'; result: unknown }
  | { status: 'ERR'; error: string };

/** One entry of an `all` fan-out result (allSettled semantics). */
export type Settled<T = unknown> = { ok: true; value: T } | { ok: false; error: unknown };

/**
 * Effects are data. A saga yields one and gets its result back on the next
 * `next()`; the interpreter (client/runtime) is the only place they execute.
 */
export type Effect =
  | { kind: 'local.query'; sql: string; vars?: Vars; epoch?: number }
  | { kind: 'local.select'; plan: QueryPlan; params?: Vars }
  | { kind: 'local.execute'; query: SealedQuery<unknown>; vars?: Vars; epoch?: number }
  | { kind: 'local.getById'; table: string; id: unknown }
  | { kind: 'local.upsert'; table: string; id: unknown; data: Record<string, unknown>; mode: 'replace' | 'merge' }
  | { kind: 'local.delete'; table: string; id: unknown }
  | { kind: 'remote.query'; sql: string; vars?: Vars; timeoutMs?: number }
  | { kind: 'ssp.register'; plan: RegisterPlan }
  | { kind: 'ssp.unregister'; hash: string }
  | { kind: 'ssp.ingest'; records: IngestRecord[] }
  | { kind: 'timer.set'; key: string; ms: number; event: RuntimeEvent }
  | { kind: 'timer.clear'; key: string }
  | { kind: 'state.read'; select: (s: ClientState) => unknown }
  | { kind: 'state.update'; fn: (s: ClientState) => ClientState }
  | { kind: 'now' }
  | { kind: 'id'; scope: 'mutation' | 'salt' }
  | { kind: 'hash'; input: string }
  | { kind: 'emit'; event: OutEvent }
  | { kind: 'dispatch'; event: RuntimeEvent }
  | { kind: 'all'; effects: Effect[] };

export type EffectKind = Effect['kind'];

/** Typed constructors. Each is one line so the saga reads like the plan. */
export const fx = {
  local: {
    query: (sql: string, vars?: Vars, epoch?: number): Effect => ({ kind: 'local.query', sql, vars, epoch }),
    select: (plan: QueryPlan, params?: Vars): Effect => ({ kind: 'local.select', plan, params }),
    execute: (query: SealedQuery<unknown>, vars?: Vars, epoch?: number): Effect => ({
      kind: 'local.execute',
      query,
      vars,
      epoch,
    }),
    getById: (table: string, id: unknown): Effect => ({ kind: 'local.getById', table, id }),
    upsert: (table: string, id: unknown, data: Record<string, unknown>, mode: 'replace' | 'merge'): Effect => ({
      kind: 'local.upsert',
      table,
      id,
      data,
      mode,
    }),
    delete: (table: string, id: unknown): Effect => ({ kind: 'local.delete', table, id }),
  },
  remote: {
    query: (sql: string, vars?: Vars, timeoutMs?: number): Effect => ({ kind: 'remote.query', sql, vars, timeoutMs }),
  },
  ssp: {
    register: (plan: RegisterPlan): Effect => ({ kind: 'ssp.register', plan }),
    unregister: (hash: string): Effect => ({ kind: 'ssp.unregister', hash }),
    ingest: (records: IngestRecord[]): Effect => ({ kind: 'ssp.ingest', records }),
  },
  timer: {
    set: (key: string, ms: number, event: RuntimeEvent): Effect => ({ kind: 'timer.set', key, ms, event }),
    clear: (key: string): Effect => ({ kind: 'timer.clear', key }),
  },
  state: {
    read: <T>(select: (s: ClientState) => T): Effect => ({ kind: 'state.read', select }),
    update: (fn: (s: ClientState) => ClientState): Effect => ({ kind: 'state.update', fn }),
  },
  now: (): Effect => ({ kind: 'now' }),
  id: (scope: 'mutation' | 'salt'): Effect => ({ kind: 'id', scope }),
  hash: (input: string): Effect => ({ kind: 'hash', input }),
  emit: (event: OutEvent): Effect => ({ kind: 'emit', event }),
  dispatch: (event: RuntimeEvent): Effect => ({ kind: 'dispatch', event }),
  all: (effects: Effect[]): Effect => ({ kind: 'all', effects }),
} as const;
