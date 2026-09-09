import { defineConfig } from 'vitest/config';

// Thresholds start at the numbers the suite hits today and are ratcheted per
// directory as the saga core lands (see the plan's commit 10). Exclusions:
//   *.fixture.ts / src/testing/**   test-only harness code
//   sqlite-worker.ts                runs inside a Worker (sqlite-wasm + OPFS +
//                                   postMessage globals); covered by the sqlite
//                                   integration tests and the build check
//   client/services.ts              constructs the real adapters (SurrealDB
//                                   WASM store, socket, SharedWorker broker,
//                                   OPFS blob cache); exercised by the example
//                                   app and the WhitePawn A/B, not by vitest
export default defineConfig({
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
    coverage: {
      provider: 'v8',
      all: true,
      reporter: ['text-summary', 'lcov'],
      include: ['src/**/*.ts'],
      exclude: [
        'src/**/*.test.ts',
        'src/**/*.d.ts',
        'src/**/*.fixture.ts',
        'src/testing/**',
        'src/services/database/sqlite-worker.ts',
        'src/client/services.ts',
      ],
      thresholds: {
        lines: 60,
        functions: 70,
        branches: 78,
        statements: 60,
        'src/kernel/**': { lines: 100, functions: 100, branches: 100, statements: 100 },
        'src/state/**': { lines: 100, functions: 100, branches: 100, statements: 100 },
        'src/query/**': { lines: 100, functions: 100, branches: 100, statements: 100 },
        'src/mutation/**': { lines: 100, functions: 100, branches: 100, statements: 100 },
        'src/sync/**': { lines: 100, functions: 100, branches: 100, statements: 100 },
        'src/boot/**': { lines: 100, functions: 100, branches: 100, statements: 100 },
      },
    },
  },
});
