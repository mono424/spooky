import { LoroDoc } from 'loro-crdt';

/**
 * Decode a `@crdt text` field's stored value into a plain-text preview.
 *
 * Accepts:
 *   - `Uint8Array` (raw LoroDoc snapshot, the shape of `@crdt`-only fields)
 *   - `{ state: Uint8Array, cursors?: ... }` (the shape of `@crdt @cursor` fields)
 *   - falsy / empty → returns ''
 *
 * Errors during import (e.g. legacy plain-string data still in the row,
 * truncated snapshot) collapse to '' so a list-row render never throws.
 *
 * Use this for read-only displays like list previews. Editing must go
 * through `useCrdtField` + `CollaborativeEditor` so the LoroDoc state is
 * the source of truth, not a derived string.
 */
export function loroPreview(value: unknown): string {
  if (!value) return '';

  // Bytes may arrive as Uint8Array, ArrayBuffer, a typed-array view, or a
  // plain `number[]` (SurrealDB serializes bytes-inside-a-FLEXIBLE-object
  // as a JSON array of byte values for the local WASM engine). Accept
  // every shape and normalize to Uint8Array.
  const asBytes = (v: unknown): Uint8Array | undefined => {
    if (v instanceof Uint8Array) return v;
    if (v instanceof ArrayBuffer) return new Uint8Array(v);
    if (ArrayBuffer.isView(v)) {
      const view = v as ArrayBufferView;
      return new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
    }
    if (Array.isArray(v) && v.length > 0 && v.every((n) => typeof n === 'number')) {
      return Uint8Array.from(v as number[]);
    }
    return undefined;
  };

  let bytes: Uint8Array | undefined;
  if (typeof value === 'object' && value !== null && !Array.isArray(value)
      && !(value instanceof Uint8Array) && !(value instanceof ArrayBuffer)
      && !ArrayBuffer.isView(value)
      && 'state' in (value as object)) {
    bytes = asBytes((value as { state?: unknown }).state);
  } else {
    bytes = asBytes(value);
  }

  if (!bytes || bytes.length === 0) return '';

  try {
    const doc = new LoroDoc();
    doc.import(bytes);
    return doc.getText('text').toString();
  } catch {
    return '';
  }
}
