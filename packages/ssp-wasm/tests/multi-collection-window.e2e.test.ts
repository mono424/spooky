/**
 * Repro for the solid-app "switching to a second collection renders 0 games"
 * bug. CollectionView registers a windowed, owner/collection-filtered view:
 *   SELECT * FROM game WHERE database = game_database:<id>
 *   ORDER BY sort_index ASC, date DESC, id ASC LIMIT 50 START 0
 * The FIRST collection opened renders fine; the SECOND comes up empty. This
 * drives the real WASM circuit directly to localize the bug to the engine.
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
  const wasmBuffer = readFileSync(join(__dirname, '../pkg/ssp_wasm_bg.wasm'));
  initSync({ module: wasmBuffer });
});

function newProcessor() {
  const p = new Sp00kyProcessor();
  // `game_database` is referenced as a record-id literal in the WHERE filter,
  // so it needs a permission too (matches the real app, which registers it).
  p.set_permissions({ game: 'true', game_database: 'true' });
  return p;
}

// A game in a given collection. `database` is a record-link id string (how the
// real records reach the SSP).
function makeGame(db: string, seq: number) {
  const id = `game:${db}_${String(seq).padStart(3, '0')}`;
  return {
    id,
    record: {
      id,
      sort_index: seq,
      date: '2024-01-01T00:00:00Z',
      database: `game_database:${db}`,
      type: 'game',
    },
  };
}

// Mirrors the real CollectionView query exactly (verified against the live
// `_00_query` table): a bound `$database` param + access/auth params, lowercase
// order keywords.
const SURQL =
  'SELECT * FROM game WHERE database = $database ORDER BY sort_index asc, date desc, id asc LIMIT 50 START 0';

function params(db: string) {
  return {
    access: 'account',
    auth: { id: 'user:test' },
    database: `game_database:${db}`,
  };
}

function windowIds(p: Sp00kyProcessor, viewId: string, db: string): string[] {
  const cfg = createViewConfig(viewId, SURQL, params(db));
  const res = p.register_view(cfg) as WasmViewUpdate;
  return (res.result_data as [string, number][]).map((r) => r[0]).sort();
}

describe('windowed view, multiple collections, registered after data exists', () => {
  it('returns each collection\'s games (2nd collection must not be empty)', () => {
    const p = newProcessor();
    for (let i = 0; i < 15; i++) {
      const a = makeGame('colA', i);
      const b = makeGame('colB', i);
      p.ingest('game', 'CREATE', a.id, a.record);
      p.ingest('game', 'CREATE', b.id, b.record);
    }

    const aIds = windowIds(p, 'view-A', 'colA');
    const bIds = windowIds(p, 'view-B', 'colB'); // 2nd registration — the repro

    expect(aIds).toHaveLength(15);
    expect(bIds).toHaveLength(15);
    // Sanity: each view only sees its own collection.
    expect(aIds.every((id) => id.startsWith('game:colA_'))).toBe(true);
    expect(bIds.every((id) => id.startsWith('game:colB_'))).toBe(true);
  });

  it('order-independent: registering colB first also returns both', () => {
    const p = newProcessor();
    for (let i = 0; i < 15; i++) {
      const a = makeGame('colA', i);
      const b = makeGame('colB', i);
      p.ingest('game', 'CREATE', a.id, a.record);
      p.ingest('game', 'CREATE', b.id, b.record);
    }
    const bIds = windowIds(p, 'view-B', 'colB');
    const aIds = windowIds(p, 'view-A', 'colA');
    expect(bIds).toHaveLength(15);
    expect(aIds).toHaveLength(15);
  });

  it('control: non-windowed WHERE view for a 2nd collection returns games', () => {
    const p = newProcessor();
    for (let i = 0; i < 15; i++) {
      const a = makeGame('colA', i);
      const b = makeGame('colB', i);
      p.ingest('game', 'CREATE', a.id, a.record);
      p.ingest('game', 'CREATE', b.id, b.record);
    }
    const view = (id: string, db: string) => {
      const cfg = createViewConfig(id, 'SELECT * FROM game WHERE database = $database', params(db));
      const res = p.register_view(cfg) as WasmViewUpdate;
      return (res.result_data as [string, number][]).length;
    };
    expect(view('plain-A', 'colA')).toBe(15);
    expect(view('plain-B', 'colB')).toBe(15);
  });
});
