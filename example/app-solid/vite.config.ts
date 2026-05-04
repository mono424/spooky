import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';
import wasm from 'vite-plugin-wasm';
import topLevelAwait from 'vite-plugin-top-level-await';
import { VitePWA } from 'vite-plugin-pwa';
import path from 'path';

export default defineConfig({
  plugins: [
    solid(),
    wasm(),
    topLevelAwait(),
    VitePWA({
      registerType: 'autoUpdate',
      includeAssets: ['favicon.png', 'apple-touch-icon.png'],
      manifest: {
        name: 'Threads',
        short_name: 'Threads',
        description: 'Collaborative offline-first thread app',
        theme_color: '#09090B',
        background_color: '#09090B',
        display: 'standalone',
        start_url: '/',
        scope: '/',
        icons: [
          { src: '/icon-192.png', sizes: '192x192', type: 'image/png' },
          { src: '/icon-512.png', sizes: '512x512', type: 'image/png' },
          { src: '/icon-512.png', sizes: '512x512', type: 'image/png', purpose: 'maskable' },
        ],
      },
      workbox: {
        // The SurrealDB WASM bundle is ~6 MB; raise the precache size cap.
        maximumFileSizeToCacheInBytes: 16 * 1024 * 1024,
        globPatterns: ['**/*.{js,css,html,svg,png,ico,wasm,woff,woff2}'],
        navigateFallback: '/index.html',
        runtimeCaching: [
          {
            urlPattern: /^https:\/\/fonts\.googleapis\.com\/.*/i,
            handler: 'CacheFirst',
            options: {
              cacheName: 'google-fonts-stylesheets',
              expiration: { maxEntries: 10, maxAgeSeconds: 60 * 60 * 24 * 365 },
            },
          },
          {
            urlPattern: /^https:\/\/fonts\.gstatic\.com\/.*/i,
            handler: 'CacheFirst',
            options: {
              cacheName: 'google-fonts-webfonts',
              expiration: { maxEntries: 30, maxAgeSeconds: 60 * 60 * 24 * 365 },
              cacheableResponse: { statuses: [0, 200] },
            },
          },
        ],
      },
      devOptions: {
        enabled: false,
      },
    }),
  ],
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
