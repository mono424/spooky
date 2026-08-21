import { defineConfig } from 'tsdown';

export default defineConfig({
  entry: ['src/index.ts'],
  format: ['esm', 'cjs'],
  dts: true,
  external: [
    'surrealdb',
    '@surrealdb/wasm',
    'solid-js',
    '@spooky-sync/core',
    '@spooky-sync/query-builder',
  ],
  clean: true,
  hash: false,
  sourcemap: true,
  target: 'es2020',
});
