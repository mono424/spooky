import { defineConfig } from 'tsdown';

export default defineConfig({
  entry: ['src/index.ts', 'src/otel/index.ts'],
  format: ['esm'],
  dts: true,
  clean: true,
  hash: false,
});
