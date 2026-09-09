import type { QueryHash } from '../types';
import type { Saga } from '../kernel/saga';
import { fx } from '../kernel/effects';
import type { QueryEntry } from '../state/client-state';
import * as R from '../state/reducers';
import { overlay } from '../state/selectors';
import type { Overlay } from '../state/selectors';
import { isWindowed, materializeEffect, rowsEqual, rowsFromResult } from './materialize';
import { buildRenderIds, resolveMembership } from './render-set';

/**
 * Render one query from state: membership (or a cold scan), the outbox
 * overlay, one local read, then notify subscribers if the rows changed. The
 * only writer of `records`. Runs on the `mat:<hash>` dedupe lane whenever the
 * query is dirty.
 */
export function* materialize(hash: QueryHash): Saga<void> {
  const [entry, ov] = (yield fx.state.read((s) => [s.queries.get(hash), overlay(s)])) as [QueryEntry | undefined, Overlay];
  if (!entry) return;
  const t0 = (yield fx.now()) as number;
  const isWindow = isWindowed(entry.def.surql);
  const membership = resolveMembership({
    phase: entry.lifecycle.phase,
    remoteArray: entry.remoteArray,
    localArray: entry.localArray,
    isWindow,
  });
  const ids = membership
    ? buildRenderIds(membership, entry.localArray, ov, {
        hasExplicitOrder: (entry.def.plan?.orderBy?.length ?? 0) > 0,
        isWindow,
      })
    : null;
  const effect = materializeEffect(entry.def, ids);
  let rows: Record<string, unknown>[];
  try {
    rows = rowsFromResult(effect, yield effect);
  } catch (error) {
    yield fx.state.update(R.compose(R.recordError(hash), R.clearDirty(hash)));
    yield fx.emit({ type: 'log', level: 'warn', message: 'materialize failed', data: { hash, error } });
    return;
  }
  const current = (yield fx.state.read((s) => s.queries.get(hash))) as QueryEntry | undefined;
  if (!current) return;
  const t1 = (yield fx.now()) as number;
  const changed = !rowsEqual(rows, current.records);
  yield fx.state.update(
    R.compose(
      R.setRecords(hash, rows, changed, t1 - t0),
      R.recordPhase(hash, 'localFetch', t1 - t0),
      changed ? R.stampUpdated(hash, t1) : (s) => s
    )
  );
  if (changed) yield fx.emit({ type: 'query:records', hash, records: rows });
}
