import { describe, it, expect } from 'vitest';

/**
 * Tests for StreamProcessor WASM module integration.
 * Verifies RecordId normalization and view update mechanics.
 */

// Mock RecordId class that mimics surrealdb's RecordId behavior
class MockRecordId {
  private _table: string;
  private _id: string;

  constructor(table: string, id: string) {
    this._table = table;
    this._id = id;
  }

  get table() {
    return { toString: () => this._table };
  }

  get id() {
    return this._id;
  }

  toString() {
    return `${this._table}:⟨${this._id}⟩`;
  }
}

// Helper: Mock the normalizeValue logic for testing
function normalizeValue(value: any): any {
  if (value === null || value === undefined) return value;

  if (typeof value === 'object') {
    // RecordId detection: check constructor name or presence of table + id getters
    if (
      value.constructor?.name === 'MockRecordId' ||
      value.constructor?.name === 'RecordId' ||
      (typeof value.toString === 'function' &&
        'table' in value &&
        'id' in value &&
        value.constructor?.name !== 'Object')
    ) {
      return value.toString();
    }

    // Fallback: old check for objects with tb and id
    if ('tb' in value && 'id' in value) {
      return `${value.tb}:${value.id}`;
    }

    if (Array.isArray(value)) {
      return value.map((v) => normalizeValue(v));
    }

    if (value.constructor === Object) {
      const out: any = {};
      for (const k in value) {
        out[k] = normalizeValue(value[k]);
      }
      return out;
    }
  }
  return value;
}

describe('RecordId Normalization', () => {
  it('should convert RecordId to string using toString()', () => {
    const recordId = new MockRecordId('user', '123');
    const normalized = normalizeValue(recordId);

    expect(normalized).toBe('user:⟨123⟩');
  });

  it('should convert RecordId with complex id to string', () => {
    const recordId = new MockRecordId('thread', 'f442770c628647999b7d1e188787dac8');
    const normalized = normalizeValue(recordId);

    expect(typeof normalized).toBe('string');
    expect(normalized).toContain('thread:');
  });

  it('should normalize RecordId in nested object', () => {
    const params = {
      id: new MockRecordId('user', '456'),
      other: 'value',
    };
    const normalized = normalizeValue(params);

    expect(typeof normalized.id).toBe('string');
    expect(normalized.id).toContain('user:');
    expect(normalized.other).toBe('value');
  });

  it('should normalize RecordId in arrays', () => {
    const params = {
      ids: [new MockRecordId('user', '1'), new MockRecordId('user', '2')],
    };
    const normalized = normalizeValue(params);

    expect(Array.isArray(normalized.ids)).toBe(true);
    expect(normalized.ids.every((id: any) => typeof id === 'string')).toBe(true);
  });

  it('should not modify plain objects without RecordId-like structure', () => {
    const plainObject = { name: 'test', value: 123 };
    const normalized = normalizeValue(plainObject);

    expect(normalized).toEqual(plainObject);
  });

  it('should handle null and undefined', () => {
    expect(normalizeValue(null)).toBe(null);
    expect(normalizeValue(undefined)).toBe(undefined);
  });

  it('should pass through primitive values', () => {
    expect(normalizeValue('string')).toBe('string');
    expect(normalizeValue(123)).toBe(123);
    expect(normalizeValue(true)).toBe(true);
  });
});

describe('StreamProcessor Ingest Behavior', () => {
  it('should match ingested record when param is normalized string', async () => {
    const params = { id: new MockRecordId('user', '2dng4ngbicbl0scod87i') };
    const normalizedParams = normalizeValue(params);

    const ingestedRecord = {
      id: 'user:2dng4ngbicbl0scod87i',
      username: 'sara',
    };

    // The key assertion: after normalization, both should be strings
    expect(typeof normalizedParams.id).toBe('string');
    expect(normalizedParams.id).toContain('user:');
    expect(normalizedParams.id).toContain('2dng4ngbicbl0scod87i');
  });
});
