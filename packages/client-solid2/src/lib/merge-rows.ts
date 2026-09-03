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
/**
 * Field-level equality for the merge. `===` is not enough: the decoder hands
 * out a fresh RecordId / Date instance for every record-link and datetime
 * column on every emission, so by identity every one of those fields "changed"
 * on every update - and a store write on an unchanged `id` re-ran everything a
 * page keyed on that row (a thread's anchor, its composer, its whole subtree).
 * Compare those wrappers by value; everything else stays identity (nested
 * objects and arrays are reconciled by their own writes).
 */
export function sameValue(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (a == null || b == null) return false;
  if (a instanceof Date && b instanceof Date) return a.getTime() === b.getTime();
  // RecordId (and the SDK's other value wrappers: Duration, Decimal, Uuid…)
  // all carry a stable `toString`; two of the same class that print alike ARE
  // the same value. Plain objects are excluded: their `toString` is
  // "[object Object]" and would make every object equal to every other.
  if (typeof a === 'object' && typeof b === 'object') {
    const ctor = (a as any).constructor;
    if (ctor && ctor === (b as any).constructor && ctor !== Object && ctor !== Array) {
      return String(a) === String(b);
    }
  }
  return false;
}

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
        if (!sameValue(reuse[field], incoming[field])) reuse[field] = incoming[field];
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
