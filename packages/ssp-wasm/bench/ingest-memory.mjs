/**
 * Wasm heap high-water mark of a reload-shaped ingest.
 *
 * Why this exists: a WhitePawn collection of ~7700 games with annotated
 * (~20 KB) PGNs parked the leader tab's wasm heap at ~1.1 GB. This measures
 * where that came from and what each fix buys: K windowed views register,
 * then ROWS land via `ingest_many` in one call (`MODE=many`), in CHUNK-sized
 * calls (`MODE=chunk`, what the client does) or per row (`MODE=row`); the same
 * rows are ingested again as UPDATEs (a re-sync); then a store-only snapshot
 * is written and, in reload order (views first), restored and reconciled.
 *
 *   node bench/ingest-memory.mjs
 *   MODE=chunk CHUNK=128 VIEWS=10 BODY=20000 node bench/ingest-memory.mjs
 *   PROJECTION=1 BODY=20000 node bench/ingest-memory.mjs
 *
 * Reference (7700 rows, 10 views, chunk 128, 20 KB bodies): heap after ingest
 * 336 MB without projection, 24 MB with; the second pass adds nothing (an
 * unchanged write is a no-op); the snapshot is 156 MB vs 1.2 MB.
 */
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { initSync, Sp00kyProcessor } from '../pkg/ssp_wasm.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const wasm = initSync({ module: readFileSync(join(__dirname, '../pkg/ssp_wasm_bg.wasm')) });
const heapMB = () => +(wasm.memory.buffer.byteLength / 1e6).toFixed(1);

const ROWS = Number(process.env.ROWS ?? 7700);
const BODY = Number(process.env.BODY ?? 776);
const VIEWS = Number(process.env.VIEWS ?? 5);
const MODE = process.env.MODE ?? 'chunk'; // many | chunk | row
const CHUNK = Number(process.env.CHUNK ?? 128);
const PAGE = 50;

const filler = '1. e4 e5 2. Nf3 Nc6 3. Bb5 a6 '.repeat(Math.ceil(BODY / 30));
const makeRow = (i) => ({
  id: `game:g${String(i).padStart(6, '0')}`,
  database: 'game_database:bench',
  white: 'player_a', black: 'player_b',
  result: i % 3 === 0 ? '1-0' : '0-1',
  date: `2026-0${(i % 9) + 1}-01T00:00:00Z`,
  speed: 'blitz', sort_index: i,
  pgn: filler.slice(0, BODY),
  _00_rv: 1,
});
const viewConfig = (id, start) => ({
  id,
  surql: `SELECT * FROM game WHERE database = 'game_database:bench' ORDER BY sort_index ASC LIMIT ${PAGE} START ${start}`,
  clientId: 'bench', ttl: '3600s', lastActiveAt: '2026-01-01T00:00:00Z',
});

const out = { ROWS, BODY, VIEWS, MODE, CHUNK, heapStart: heapMB() };
let p = new Sp00kyProcessor();
p.set_permissions({ game: 'true' });
if (process.env.PROJECTION === '1') p.set_projection(true);
for (let v = 0; v < VIEWS; v++) p.register_view(viewConfig(`w${v}`, v * PAGE));
out.heapAfterRegister = heapMB();

const rows = Array.from({ length: ROWS }, (_, i) => makeRow(i));
let t0 = performance.now();
if (MODE === 'many') {
  p.ingest_many(rows.map((r) => ({ table: 'game', op: 'CREATE', id: r.id, record: r })));
} else if (MODE === 'chunk') {
  for (let i = 0; i < ROWS; i += CHUNK) {
    p.ingest_many(rows.slice(i, i + CHUNK).map((r) => ({ table: 'game', op: 'CREATE', id: r.id, record: r })));
  }
} else {
  for (const r of rows) p.ingest('game', 'CREATE', r.id, r);
}
out.ingestMs = Math.round(performance.now() - t0);
out.heapAfterIngest = heapMB();

// second ingest of the same rows as UPDATE (what a re-sync does): does the heap grow again?
t0 = performance.now();
p.ingest_many(rows.map((r) => ({ table: 'game', op: 'UPDATE', id: r.id, record: r })));
out.reingestMs = Math.round(performance.now() - t0);
out.heapAfterReingest = heapMB();

out.deadMB = +(p.dead_bytes() / 1e6).toFixed(1);
out.liveMB = +(p.live_bytes() / 1e6).toFixed(1);
t0 = performance.now();
const state = p.save_store_state();
out.saveMs = Math.round(performance.now() - t0);
out.snapshotMB = +(state.length / 1e6).toFixed(1);
out.heapAfterSave = heapMB();

p.free();
p = new Sp00kyProcessor();
p.set_permissions({ game: 'true' });
if (process.env.PROJECTION === '1') p.set_projection(true);
// reload order: views register first, then the snapshot lands
for (let v = 0; v < VIEWS; v++) p.register_view(viewConfig(`r${v}`, v * PAGE));
t0 = performance.now();
const ups = p.load_store_state(state);
out.loadMs = Math.round(performance.now() - t0);
out.loadUpdates = ups.length;
out.heapAfterLoad = heapMB();
t0 = performance.now();
const rec = p.reconcile('game', rows.map((r) => [r.id, 1]));
out.reconcileMs = Math.round(performance.now() - t0);
out.reconcileFetch = rec.fetch.length;
out.heapEnd = heapMB();
out.rssMB = Math.round(process.memoryUsage().rss / 1e6);
console.log(JSON.stringify(out));
