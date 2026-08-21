import { defineConfig } from 'vitest/config';

export default defineConfig({
  // solid-js's `node` export condition resolves the SSR build, where user
  // effects intentionally never run. Force the browser dev build so tests
  // exercise real client-side reactivity semantics.
  resolve: {
    conditions: ['browser', 'development'],
  },
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
  },
});
