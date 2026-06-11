// Build-time string constants injected by tsdown's `define` (see
// tsdown.config.ts). They carry the real bundled frontend package versions so
// the DevTools can surface frontend/backend version drift.
//
// Typed as `string | undefined` on purpose: the substitution only happens when
// `@spooky-sync/core` is built with our tsdown plugin. A downstream app that
// bundles core from source never runs that plugin, so these identifiers must be
// guarded with `typeof` (see modules/devtools/index.ts) to avoid a runtime
// ReferenceError that would break DB initialization.
declare const __SP00KY_CORE_VERSION__: string | undefined;
declare const __SP00KY_WASM_VERSION__: string | undefined;
declare const __SP00KY_SURREAL_VERSION__: string | undefined;
