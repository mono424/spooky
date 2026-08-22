import { snapshot } from 'solid-js';

/**
 * Keyed in-place merge of a fresh row list into a store draft array.
 *
 * Why not `reconcile(rows, 'id')`: in Solid 2 rc.1 a reconcile REPLACES the row
 * objects, so a component that captured `data()[0]` sees a different object
 * after the next emission, and anything keyed on row identity (`<For>` without
 * an explicit key, a memo comparing rows) re-creates its subtree on every live
 * update. Mutating the draft in place keeps the identity AND keeps updates
 * fine-grained: only the fields that actually changed are written, so a row
 * whose data is unchanged notifies nobody.
 *
 * Rows are matched by `key` (default `id`); unmatched incoming rows are
 * inserted as-is and trailing rows are dropped, so add / remove / reorder all
 * reach coarse readers.
 */
export function mergeRows(draft: any[], next: any[], key = 'id'): void {
  const byKey = new Map<any, any>();
  for (const row of draft) {
    const k = row?.[key];
    if (k !== undefined) byKey.set(String(k), row);
  }

  for (let i = 0; i < next.length; i++) {
    const incoming = next[i];
    const k = incoming?.[key];
    const reuse = k !== undefined ? byKey.get(String(k)) : undefined;

    if (reuse) {
      for (const field of Object.keys(incoming)) {
        if (reuse[field] !== incoming[field]) reuse[field] = incoming[field];
      }
      // `snapshot` first: iterating the proxy's own keys while deleting from it
      // is not safe, and the raw object is what carries the stale fields.
      for (const field of Object.keys(snapshot(reuse))) {
        if (!(field in incoming)) delete reuse[field];
      }
      if (draft[i] !== reuse) draft[i] = reuse;
    } else if (draft[i] !== incoming) {
      draft[i] = incoming;
    }
  }

  if (draft.length > next.length) draft.splice(next.length);
}
