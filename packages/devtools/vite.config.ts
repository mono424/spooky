import { defineConfig } from 'vite';
import { resolve } from 'path';

export default defineConfig({
  build: {
    outDir: 'dist',
    rollupOptions: {
      input: {
        devtools: resolve(__dirname, 'src/devtools.ts'),
        panel: resolve(__dirname, 'src/panel.ts'),
        background: resolve(__dirname, 'src/background.ts'),
        content: resolve(__dirname, 'src/content.ts'),
      },
      output: {
        entryFileNames: '[name].js',
        format: 'iife',
      },
    },
    minify: false,
    sourcemap: true,
  },
  publicDir: 'public',
});
