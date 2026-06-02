import { defineConfig } from 'tsdown';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);

// Resolve real package versions at build time so the DevTools can report the
// actual bundled frontend versions (used to detect frontend/backend drift).
const corePkg = require('./package.json');
const coreVersion: string = corePkg.version;
// ssp-wasm has no `exports` map; resolve its package.json directly, falling
// back to the relative workspace path if module resolution can't find it.
let wasmVersion: string;
try {
  wasmVersion = require('@spooky-sync/ssp-wasm/package.json').version;
} catch {
  wasmVersion = require('../ssp-wasm/package.json').version;
}
// `surrealdb`'s `exports` map blocks importing its package.json, so read the
// bundled client version from our own pinned dependency declaration instead.
const surrealVersion: string = String(corePkg.dependencies?.surrealdb ?? 'unknown').replace(
  /^[\^~]/,
  ''
);

// tsdown 0.12.x forwards a top-level `define` to rolldown's inputOptions, which
// rejects it — so we do the substitution ourselves with a tiny transform
// plugin. Only our own modules reference the `__SP00KY_*__` identifiers, and
// the early `includes` check skips everything else.
const replacements: Record<string, string> = {
  __SP00KY_CORE_VERSION__: JSON.stringify(coreVersion),
  __SP00KY_WASM_VERSION__: JSON.stringify(wasmVersion),
  __SP00KY_SURREAL_VERSION__: JSON.stringify(surrealVersion),
};

const versionDefinePlugin = {
  name: 'sp00ky-version-define',
  transform(code: string) {
    if (!code.includes('__SP00KY_')) return null;
    let out = code;
    for (const [token, value] of Object.entries(replacements)) {
      out = out.split(token).join(value);
    }
    return out;
  },
};

export default defineConfig({
  entry: ['src/index.ts', 'src/otel/index.ts'],
  format: ['esm'],
  dts: true,
  clean: true,
  hash: false,
  plugins: [versionDefinePlugin],
});
