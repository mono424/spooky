/**
 * Cost of `Circuit::save` relative to store size.
 *
 * Why this exists: the browser client used to snapshot the circuit on every
 * ingest batch AND every query register/unregister. `Circuit::save` deep-clones
 * the whole store (every row of every ingested table, full bodies) plus every
 * view cache and JSON-encodes the result, so the cost scales with everything the
 * session has ever ingested. On a windowed list over a few thousand rows that
 * came to hundreds of whole-store serializations for a single scroll, and it
 * pushed a renderer into a V8 out-of-memory abort (Chrome sad tab, "Error
 * code: 5" = SIGTRAP on macOS).
 *
 * Snapshots are now opt-in (`persistCircuit`) and checkpointed. This benchmark
 * is the receipt: run both modes and compare.
 *
 *   node bench/circuit-save.mjs withSave   # the old per-batch behavior
 *   node bench/circuit-save.mjs noSave     # current default
 *
 * The shape mirrors a real WhitePawn collection page: one live query per 50-row
 * window, registered and unregistered as the list scrolls, over ~3.7k rows whose
 * dominant field is a ~780-byte PGN string.
 */
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { initSync, Sp00kyProcessor } from '../pkg/ssp_wasm.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const wasm = initSync({ module: readFileSync(join(__dirname, '../pkg/ssp_wasm_bg.wasm')) });

const MODE = process.argv[2] ?? 'withSave';
const WITH_SAVE = MODE !== 'noSave';

const ROWS = Number(process.env.BENCH_ROWS ?? 3728);
const BODY_BYTES = Number(process.env.BENCH_BODY_BYTES ?? 776);
const PAGE = 50;
const WINDOW_KEEP = 5; // near-visible windows held open at once

const heap = () => wasm.memory.buffer.byteLength;

function makeRow(i) {
  const filler = '1. e4 e5 2. Nf3 Nc6 3. Bb5 a6 '.repeat(Math.ceil(BODY_BYTES / 30));
  return {
    id: `game:g${String(i).padStart(6, '0')}`,
    database: 'game_database:bench',
    white: 'player_a',
    black: 'player_b',
    result: i % 3 === 0 ? '1-0' : '0-1',
    date: `2026-0${(i % 9) + 1}-01T00:00:00Z`,
    speed: 'blitz',
    sort_index: i,
    pgn: filler.slice(0, BODY_BYTES),
  };
}

const processor = new Sp00kyProcessor();
processor.set_permissions({ game: 'true' });

let saveCalls = 0;
let saveBytes = 0;
let saveMs = 0;
let worstSaveMs = 0;
let lastStateLen = 0;

function snapshot() {
  if (!WITH_SAVE) return;
  const t0 = performance.now();
  const state = processor.save_state();
  saveMs += performance.now() - t0;
  worstSaveMs = Math.max(worstSaveMs, performance.now() - t0);
  saveCalls++;
  lastStateLen = state.length;
  // The browser then re-encoded it (LocalStoragePersistenceClient JSON.stringifies
  // an already-JSON string) and wrote it synchronously on the main thread. Both
  // strings land in V8's large-object space.
  saveBytes += state.length + JSON.stringify(state).length;
}

const viewConfig = (id, start) => ({
  id,
  surql: `SELECT * FROM game WHERE database = 'game_database:bench' ORDER BY sort_index ASC LIMIT ${PAGE} START ${start}`,
  clientId: 'bench',
  ttl: '3600s',
  lastActiveAt: new Date('2026-01-01T00:00:00Z').toISOString(),
});

const heapStart = heap();
const wallStart = performance.now();
const windows = Math.ceil(ROWS / PAGE);
const samples = [];

for (let w = 0; w < windows; w++) {
  processor.register_view(viewConfig(`bench_window_${w}`, w * PAGE));
  snapshot();

  for (let i = 0; i < PAGE; i++) {
    const n = w * PAGE + i;
    if (n >= ROWS) break;
    const row = makeRow(n);
    processor.ingest('game', 'CREATE', row.id, row);
  }
  snapshot();

  if (w >= WINDOW_KEEP) {
    processor.unregister_view(`bench_window_${w - WINDOW_KEEP}`);
    snapshot();
  }

  if (w % 15 === 0 || w === windows - 1) {
    samples.push({
      window: w,
      rows: Math.min((w + 1) * PAGE, ROWS),
      snapshotMB: +(lastStateLen / 1e6).toFixed(2),
      wasmMB: +(heap() / 1e6).toFixed(1),
      serializedMB: +(saveBytes / 1e6).toFixed(0),
      elapsedS: +((performance.now() - wallStart) / 1000).toFixed(1),
    });
  }
}

const wallS = (performance.now() - wallStart) / 1000;

console.log(`\nmode=${MODE}  rows=${ROWS}  page=${PAGE}  windows=${windows}\n`);
console.table(samples);
console.log({
  mode: MODE,
  saveStateCalls: saveCalls,
  finalSnapshotMB: +(lastStateLen / 1e6).toFixed(2),
  totalSerializedGB: +(saveBytes / 1e9).toFixed(2),
  timeInSaveStateS: +(saveMs / 1000).toFixed(1),
  worstSingleSaveMs: +worstSaveMs.toFixed(0),
  totalWallS: +wallS.toFixed(1),
  wasmHeapStartMB: +(heapStart / 1e6).toFixed(1),
  wasmHeapEndMB: +(heap() / 1e6).toFixed(1),
  rssMB: +(process.memoryUsage().rss / 1e6).toFixed(0),
});
