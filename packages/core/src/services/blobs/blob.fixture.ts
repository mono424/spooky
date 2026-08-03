/**
 * Test-only doubles for the blob cache. Follows the repo's convention of
 * hand-rolled fakes at the module boundary (cf. `sqlite-transport.fixture.ts`)
 * rather than pulling in a DOM or a fake-indexeddb: `BlobCache` is pure logic
 * over `BlobStore` + `LocalStore`, so both can be faked in node.
 */
import { RecordId } from 'surrealdb';
import type { Id, LocalStore, Row } from '../database/cache-engine';

/** A `LocalStore` with only the three verbs `BlobManifest` uses. */
export interface FakeLocalStore {
  store: LocalStore;
  rows: Map<string, Row>;
  /** Counts, so a test can assert the manifest batched instead of write-storming. */
  upserts: number;
  deletes: number;
  /** Make every read fail — the memory-fallback / wiped-store case. */
  failReads: boolean;
}

function idOf(id: Id): string {
  return id instanceof RecordId ? String(id.id) : String(id);
}

export function fakeLocalStore(): FakeLocalStore {
  const state: FakeLocalStore = {
    rows: new Map(),
    upserts: 0,
    deletes: 0,
    failReads: false,
    store: null as unknown as LocalStore,
  };

  state.store = {
    async selectByIds(_table: string, ids: Id[]): Promise<Row[]> {
      if (state.failReads) throw new Error('local store unavailable');
      const out: Row[] = [];
      for (const id of ids) {
        const key = idOf(id);
        const row = state.rows.get(key);
        if (row) out.push({ ...row, id: new RecordId('_00_blob', key) });
      }
      return out;
    },
    async upsert(_table: string, id: Id, data: Row): Promise<void> {
      state.upserts++;
      state.rows.set(idOf(id), { ...data });
    },
    async delete(_table: string, id: Id): Promise<void> {
      state.deletes++;
      state.rows.delete(idOf(id));
    },
  } as unknown as LocalStore;

  return state;
}

/** A logger that satisfies the `Logger` shape without printing. */
export function silentLogger(): any {
  const noop = () => {};
  const logger: any = { debug: noop, info: noop, warn: noop, error: noop, fatal: noop, trace: noop };
  logger.child = () => logger;
  return logger;
}

/** Deterministic object-URL factory: no DOM needed, and revokes are countable. */
export function fakeUrls() {
  let next = 0;
  const live = new Set<string>();
  const revoked: string[] = [];
  return {
    live,
    revoked,
    factory: {
      create: (_blob: Blob) => {
        const url = `blob:fake/${next++}`;
        live.add(url);
        return url;
      },
      revoke: (url: string) => {
        live.delete(url);
        revoked.push(url);
      },
    },
  };
}

export function bytes(size: number, fill = 'a'): Blob {
  return new Blob([fill.repeat(size)]);
}
