import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// Regression guard for the auth-callback ordering bug we fixed in
// `Sp00kyClient.init()`. The contract:
//
//   this.auth.subscribe(async (userId) => {
//     this.dataModule.setCurrentUserId(userId);   // ← must be first
//     // ... awaits and other work ...
//   });
//
// If any future refactor moves `setCurrentUserId` after the first
// `await`, the AuthProvider's sibling subscriber gets to run with a
// stale `dataModule.currentUserId` and registers queries against the
// wrong `_00_query[_user_*]` / `_00_list_ref_user_<id>` tables.
//
// A runtime mock test for this is heavy — Sp00kyClient drags in
// SurrealDB, the WASM SSP, the persistence client, etc. A
// structural regex test is enough: it catches the only failure mode
// (someone moves the synchronous setCurrentUserId call) without
// needing the full graph.

describe('Sp00kyClient.auth.subscribe ordering invariant', () => {
  const sourcePath = resolve(__dirname, 'sp00ky.ts');
  const source = readFileSync(sourcePath, 'utf-8');

  it('source file is readable', () => {
    expect(source.length).toBeGreaterThan(0);
  });

  it('auth.subscribe sets currentUserId before any await', () => {
    // Find the auth.subscribe arrow body. Match the contents of the
    // outermost `{}` after `this.auth.subscribe(async (userId) => `.
    const match = source.match(
      /this\.auth\.subscribe\(\s*async\s*\(\s*userId\s*[^)]*\)\s*=>\s*\{([\s\S]*?)\n {6}\}\s*\)/
    );
    expect(
      match,
      'expected an auth.subscribe(async (userId) => { ... }) block in sp00ky.ts'
    ).not.toBeNull();

    const body = match![1];

    // Strip line comments so a stray `//` doesn't trick the regex.
    const stripped = body
      .split('\n')
      .map((line) => line.replace(/\/\/.*$/, ''))
      .join('\n');

    const setUserIdIdx = stripped.indexOf('this.dataModule.setCurrentUserId(userId)');
    expect(
      setUserIdIdx,
      'dataModule.setCurrentUserId(userId) must appear in the auth.subscribe body'
    ).toBeGreaterThanOrEqual(0);

    const firstAwaitIdx = stripped.search(/\bawait\b/);
    if (firstAwaitIdx >= 0) {
      expect(
        setUserIdIdx,
        `dataModule.setCurrentUserId(userId) (idx ${setUserIdIdx}) must come BEFORE the first \`await\` (idx ${firstAwaitIdx}) so the AuthProvider's sibling subscriber sees a fresh user id when it fires synchronously after our callback. Moving the call past an \`await\` reintroduces the stale-user-id race documented in docs/surrealdb-bugs/ if a related gap is found, and the prior session's retrospective.`
      ).toBeLessThan(firstAwaitIdx);
    }
  });
});
