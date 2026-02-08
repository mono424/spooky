import { defineConfig } from 'tsdown';

export default defineConfig({
  entry: ['src/index.ts'],
  format: ['esm', 'cjs'],
  dts: true,
  external: [
    'surrealdb',
    '@surrealdb/wasm',
    'solid-js',
    '@spooky/core',
    '@spooky/query-builder',
    'valtio',
  ],
  clean: true,
  hash: false,
  sourcemap: true,
  target: 'es2020',
});
