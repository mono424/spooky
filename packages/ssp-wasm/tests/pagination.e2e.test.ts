/**
 * E2E pagination tests — `LIMIT n START m` windowing through the real WASM
 * circuit.
 *
 * Reproduces the solid-app game-list infinite scroll: one stable view per page
 * window (`LIMIT 30 START page*30`) registered against a single shared
 * processor, ordered `sort_index ASC, date DESC`. The pages must be complete,
 * non-overlapping, and correctly ordered — otherwise the app's scroll stalls
 * after the first page (a short/empty page flips `lastPageFull` to false and
 * growth stops).
 *
 * `START` had no prior e2e coverage.
 */

import { describe, it, expect, beforeAll } from 'vitest';
import { readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { initSync, Sp00kyProcessor } from '../pkg/ssp_wasm.js';
import type { WasmViewUpdate } from '../pkg/ssp_wasm';
import { createViewConfig } from './helpers';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

beforeAll(() => {
  const wasmPath = join(__dirname, '../pkg/ssp_wasm_bg.wasm');
  const wasmBuffer = readFileSync(wasmPath);
  initSync({ module: wasmBuffer });
});

// A game row mirroring the schema fields the list query touches.
function makeGame(seq: number, sortIndex: number, date: string) {
  const id = `game:${String(seq).padStart(4, '0')}`;
  return { id, record: { id, sort_index: sortIndex, date, type: 'game' } };
}

const PAGE = 30;
const ORDER = 'ORDER BY sort_index ASC, date DESC';

// Seed a permissive `select` permission so register_view isn't default-denied.
function newProcessor() {
  const processor = new Sp00kyProcessor();
  processor.set_permissions({ game: 'true' });
  return processor;
}

describe('LIMIT/START pagination (infinite scroll windows)', () => {
  it('pages a large set into complete, non-overlapping windows', () => {
    const processor = newProcessor();

    // 65 games, distinct sort_index 0..64 → global order is game:0000..game:0064.
    const games = Array.from({ length: 65 }, (_, i) =>
      makeGame(i, i, '2024-01-01T00:00:00Z')
    );
    for (const g of games) processor.ingest('game', 'CREATE', g.id, g.record);

    const pageIds = (start: number) => {
      const cfg = createViewConfig(
        `page-${start}`,
        `SELECT * FROM game ${ORDER} LIMIT ${PAGE} START ${start}`
      );
      const res = processor.register_view(cfg) as WasmViewUpdate;
      return (res.result_data as [string, number][]).map((r) => r[0]).sort();
    };

    const p0 = pageIds(0);
    const p1 = pageIds(30);
    const p2 = pageIds(60);

    // Page 1 (START 30) is the window that determines whether scroll advances
    // past the first page. It must be a full window of the *next* 30 rows.
    expect(p0).toHaveLength(30);
    expect(p1).toHaveLength(30);
    expect(p2).toHaveLength(5); // short tail = real end

    expect(p0).toEqual(games.slice(0, 30).map((g) => g.id).sort());
    expect(p1).toEqual(games.slice(30, 60).map((g) => g.id).sort());
    expect(p2).toEqual(games.slice(60, 65).map((g) => g.id).sort());

    // No overlap between adjacent windows.
    expect(p0.filter((id) => p1.includes(id))).toEqual([]);
    expect(p1.filter((id) => p2.includes(id))).toEqual([]);
  });

  it('orders a tied sort_index window by date DESC (newest first)', () => {
    const processor = newProcessor();

    // All share sort_index 0 (pre-migration default / a tie), so `date DESC`
    // decides. `date` is a SurrealDB datetime → reaches the SSP as a string.
    const games = [
      makeGame(0, 0, '2020-01-01T00:00:00Z'),
      makeGame(1, 0, '2021-01-01T00:00:00Z'),
      makeGame(2, 0, '2022-01-01T00:00:00Z'),
      makeGame(3, 0, '2023-01-01T00:00:00Z'),
      makeGame(4, 0, '2024-01-01T00:00:00Z'),
    ];
    for (const g of games) processor.ingest('game', 'CREATE', g.id, g.record);

    const cfg = createViewConfig('page-tied', `SELECT * FROM game ${ORDER} LIMIT 2 START 0`);
    const res = processor.register_view(cfg) as WasmViewUpdate;
    const ids = (res.result_data as [string, number][]).map((r) => r[0]).sort();

    // Newest two: 2024 (game:0004) and 2023 (game:0003).
    expect(ids).toEqual(['game:0003', 'game:0004']);
  });
});
