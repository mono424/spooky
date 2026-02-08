import { defineConfig } from 'tsdown';

export default defineConfig({
  entry: ['src/index.ts'],
  format: ['esm', 'cjs'],
  dts: true,
  external: ['surrealdb'],
  clean: true,
  hash: false,
  sourcemap: true,
});
