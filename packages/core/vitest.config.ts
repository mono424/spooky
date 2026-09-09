import { defineConfig } from 'vitest/config';

// Thresholds start at the numbers the suite hits today and are ratcheted per
// directory as the saga core lands (see the plan's commit 10). Exclusions:
//   *.fixture.ts / src/testing/**   test-only harness code
//   sqlite-worker.ts                runs inside a Worker (sqlite-wasm + OPFS +
//                                   postMessage globals); covered by the sqlite
//                                   integration tests and the build check
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
      ],
      thresholds: {
        lines: 60,
        functions: 70,
        branches: 78,
        statements: 60,
      },
    },
  },
});
