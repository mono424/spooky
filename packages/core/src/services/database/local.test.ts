import { describe, it, expect } from 'vitest';
import { isLocalStoreOpenError } from './local';

// `LocalDatabaseService.connect` recovers (drops the store + reconnects, falling
// back to in-memory) only when the failure is the SurrealDB-WASM IndexedDB
// open/transaction error — NOT for unrelated errors. This pins the message match
// against the real error the engine throws so a recovery path doesn't silently
// stop triggering (or start swallowing unrelated failures).
describe('isLocalStoreOpenError', () => {
  it('matches the real SurrealDB-WASM IndexedDB key-value store error', () => {
    const real = new Error(
      'There was a problem with the key-value store: There was a problem with a ' +
        'transaction: An IndexedDB error occured: idb error'
    );
    expect(isLocalStoreOpenError(real)).toBe(true);
  });

  it('matches on the individual signals (indexeddb / idb error / key-value store)', () => {
    expect(isLocalStoreOpenError(new Error('IndexedDB is not available'))).toBe(true);
    expect(isLocalStoreOpenError(new Error('idb error'))).toBe(true);
    expect(isLocalStoreOpenError(new Error('problem with the key-value store'))).toBe(true);
    // Non-Error inputs are stringified.
    expect(isLocalStoreOpenError('An IndexedDB error occured')).toBe(true);
  });

  it('does NOT match unrelated errors (so we never clear the store spuriously)', () => {
    expect(isLocalStoreOpenError(new Error('WebSocket connection refused'))).toBe(false);
    expect(isLocalStoreOpenError(new Error('Parse error: unexpected token'))).toBe(false);
    expect(isLocalStoreOpenError(new Error('permission denied'))).toBe(false);
    expect(isLocalStoreOpenError(null)).toBe(false);
    expect(isLocalStoreOpenError(undefined)).toBe(false);
  });
});

// Per-user local buckets: URL/name derivation and the SCOPE of the tier-2
// corruption drop. The drop must only ever match the failing bucket's store —
// a substring wipe would take every user's cache AND their un-pushed mutation
// outboxes down with one corrupt store.
import { bucketStoreName, bucketStoreUrl, matchesBucketStore } from './local';

describe('bucket store naming', () => {
  it('derives the store url/name from the bucket id', () => {
    expect(bucketStoreUrl('abc')).toBe('indxdb://sp00ky-abc');
    expect(bucketStoreName('abc')).toBe('sp00ky-abc');
    expect(bucketStoreUrl('anon')).toBe('indxdb://sp00ky-anon');
  });
});

describe('matchesBucketStore', () => {
  it('matches the exact store and derived names', () => {
    expect(matchesBucketStore('sp00ky-abc', 'sp00ky-abc')).toBe(true);
    expect(matchesBucketStore('sp00ky-abc-wal', 'sp00ky-abc')).toBe(true);
    expect(matchesBucketStore('surrealdb/sp00ky-abc', 'sp00ky-abc')).toBe(true);
    expect(matchesBucketStore('SP00KY-ABC', 'sp00ky-abc')).toBe(true);
  });

  it("never matches another bucket's store", () => {
    expect(matchesBucketStore('sp00ky-abcdef', 'sp00ky-abc')).toBe(false);
    expect(matchesBucketStore('sp00ky-anon', 'sp00ky-abc')).toBe(false);
    expect(matchesBucketStore('sp00ky', 'sp00ky-abc')).toBe(false);
    // The legacy shared store is not any bucket's store.
    expect(matchesBucketStore('sp00ky', 'sp00ky-anon')).toBe(false);
  });
});
