import { defineConfig } from 'vite';
import { resolve } from 'path';
import { builtinModules } from 'module';

// Bundles the devtools MCP server into a single self-contained file that ships
// inside the @spooky-sync/cli tarball. The CLI package declares no runtime
// dependencies, so the server's own deps (@modelcontextprotocol/sdk, ws, zod)
// have to be inlined here, otherwise `spky mcp serve` fails on an installed
// copy with ERR_MODULE_NOT_FOUND.
const entry = resolve(__dirname, '../devtools-mcp/src/index.ts');

export default defineConfig({
  build: {
    ssr: true,
    target: 'node20',
    outDir: resolve(__dirname, 'devtools-mcp'),
    emptyOutDir: true,
    minify: false,
    lib: {
      entry,
      formats: ['es'],
      fileName: () => 'index.js',
    },
    rollupOptions: {
      // Node builtins stay external; ws probes these native speedups behind a
      // try/catch and works fine without them.
      external: [
        ...builtinModules,
        ...builtinModules.map((m) => `node:${m}`),
        'bufferutil',
        'utf-8-validate',
      ],
      output: {
        // The shebang already comes through from devtools-mcp/src/index.ts.
        inlineDynamicImports: true,
      },
    },
  },
});
