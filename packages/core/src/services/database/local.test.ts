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
