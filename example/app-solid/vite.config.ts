import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';
import path from 'path';

export default defineConfig({
  plugins: [solid()],
  resolve: {
    alias: {
      '@spooky-sync/client-solid': path.resolve(__dirname, '../../packages/client-solid/src/index.ts'),
      '@spooky-sync/core/otel': path.resolve(__dirname, '../../packages/core/src/otel/index.ts'),
      '@spooky-sync/core': path.resolve(__dirname, '../../packages/core/src/index.ts'),
      '@spooky-sync/query-builder': path.resolve(__dirname, '../../packages/query-builder/src/index.ts'),
    },
  },
  server: {
    port: 3006,
    proxy: {
      '/v1/logs': {
        target: 'http://localhost:4318',
        changeOrigin: true,
        secure: false,
      },
    },
  },
  build: {
    target: 'esnext',
  },
  optimizeDeps: {
    exclude: ['@surrealdb/wasm'],
    esbuildOptions: {
      target: 'esnext',
    },
  },
  esbuild: {
    supported: {
      'top-level-await': true,
    },
  },
});
