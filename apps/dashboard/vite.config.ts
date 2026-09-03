import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';

export default defineConfig({
  plugins: [solid()],
  // The scheduler serves this bundle under /admin, so every asset URL has to
  // be prefixed. Relative would also work embedded but breaks the SPA fallback
  // for a deep link like /admin/workflows/abc, where the browser resolves
  // assets against /admin/workflows/.
  base: '/admin/',
  build: {
    target: 'es2020',
    outDir: 'dist',
    emptyOutDir: true,
    // One chunk. The dashboard is small and is served from a scheduler that
    // may be behind a slow link; a single request beats a waterfall.
    chunkSizeWarningLimit: 1200,
  },
  server: {
    port: 4300,
  },
});
