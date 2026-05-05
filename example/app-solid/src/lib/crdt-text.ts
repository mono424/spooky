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

  let bytes: Uint8Array | undefined;
  if (value instanceof Uint8Array) {
    bytes = value;
  } else if (value instanceof ArrayBuffer) {
    bytes = new Uint8Array(value);
  } else if (ArrayBuffer.isView(value)) {
    const view = value as ArrayBufferView;
    bytes = new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
  } else if (typeof value === 'object' && 'state' in (value as object)) {
    const s = (value as { state?: unknown }).state;
    if (s instanceof Uint8Array) bytes = s;
    else if (s instanceof ArrayBuffer) bytes = new Uint8Array(s);
    else if (ArrayBuffer.isView(s)) {
      const view = s as ArrayBufferView;
      bytes = new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
    }
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
