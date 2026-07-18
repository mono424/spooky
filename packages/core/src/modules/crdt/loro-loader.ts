/**
 * Lazy, cached loader for `loro-crdt`. Keeping loro behind a dynamic `import()`
 * (instead of a static top-level import in `crdt-field.ts`, which is re-exported
 * from the package entry) breaks the static graph edge, so the loro chunk only
 * ships to apps that actually use CRDT fields.
 *
 * `preloadLoro()` is fired from the client constructor when `config.crdt` is on,
 * so the chunk is fetched at page load and the first `openCrdtField` doesn't
 * block on a network round-trip. `loadLoro()` kicks the same import if it hasn't
 * started, so opening a field still works when the flag is off.
 */
type LoroModule = typeof import('loro-crdt');

let loroPromise: Promise<LoroModule> | null = null;

/** Start (or return the in-flight/cached) `loro-crdt` import. Fire-and-forget. */
export function preloadLoro(): Promise<LoroModule> {
  loroPromise ??= import('loro-crdt');
  return loroPromise;
}

/** Await the loro module, starting the import if `preloadLoro` hasn't run. */
export function loadLoro(): Promise<LoroModule> {
  return preloadLoro();
}
