import { describe, it, expect } from 'vitest';
import { RecordId } from 'surrealdb';
import { parseParams, cleanRecord } from './parser';

describe('parseParams', () => {
  it('passes through plain values unchanged', () => {
    const schema = {
      name: { type: 'string' as const, optional: false },
      age: { type: 'number' as const, optional: false },
    };
    const result = parseParams(schema, { name: 'Alice', age: 30 });
    expect(result).toEqual({ name: 'Alice', age: 30 });
  });

  it('converts string to RecordId for recordId columns', () => {
    const schema = {
      owner: { type: 'string' as const, optional: false, recordId: true },
    };
    const result = parseParams(schema, { owner: 'user:123' });
    expect(result.owner).toBeInstanceOf(RecordId);
    expect(result.owner.table.toString()).toBe('user');
    expect(result.owner.id).toBe('123');
  });

  it('passes through existing RecordId for recordId columns', () => {
    const schema = {
      owner: { type: 'string' as const, optional: false, recordId: true },
    };
    const rid = new RecordId('user', '456');
    const result = parseParams(schema, { owner: rid });
    expect(result.owner).toBe(rid);
  });

  it('converts string to Date for dateTime columns', () => {
    const schema = {
      createdAt: { type: 'string' as const, optional: false, dateTime: true },
    };
    const result = parseParams(schema, { createdAt: '2024-01-01T00:00:00Z' });
    expect(result.createdAt).toBeInstanceOf(Date);
    expect(result.createdAt.toISOString()).toBe('2024-01-01T00:00:00.000Z');
  });

  it('converts number (timestamp) to Date for dateTime columns', () => {
    const schema = {
      createdAt: { type: 'string' as const, optional: false, dateTime: true },
    };
    const ts = 1704067200000; // 2024-01-01T00:00:00.000Z
    const result = parseParams(schema, { createdAt: ts });
    expect(result.createdAt).toBeInstanceOf(Date);
    expect(result.createdAt.getTime()).toBe(ts);
  });

  it('passes through existing Date for dateTime columns', () => {
    const schema = {
      createdAt: { type: 'string' as const, optional: false, dateTime: true },
    };
    const date = new Date('2024-01-01');
    const result = parseParams(schema, { createdAt: date });
    expect(result.createdAt).toBe(date);
  });

  it('skips undefined values', () => {
    const schema = {
      name: { type: 'string' as const, optional: false },
      age: { type: 'number' as const, optional: true },
    };
    const result = parseParams(schema, { name: 'Alice', age: undefined });
    expect(result).toEqual({ name: 'Alice' });
    expect('age' in result).toBe(false);
  });

  it('throws on invalid recordId value', () => {
    const schema = {
      owner: { type: 'string' as const, optional: false, recordId: true },
    };
    expect(() => parseParams(schema, { owner: 12345 })).toThrow('Invalid value for owner');
  });

  it('throws on invalid dateTime value', () => {
    const schema = {
      createdAt: { type: 'string' as const, optional: false, dateTime: true },
    };
    expect(() => parseParams(schema, { createdAt: true })).toThrow(
      'Invalid value for createdAt'
    );
  });
});

describe('cleanRecord', () => {
  const tableSchema = {
    name: { type: 'string' as const, optional: false },
    age: { type: 'number' as const, optional: false },
  };

  it('keeps schema fields and strips non-schema fields', () => {
    const record = { id: 'user:1', name: 'Alice', age: 30, _internal: true, computed_score: 99 };
    const result = cleanRecord(tableSchema, record);
    expect(result).toEqual({ id: 'user:1', name: 'Alice', age: 30 });
  });

  it('always preserves id even though it is not in schema', () => {
    const record = { id: 'user:2', extra: 'gone' };
    const result = cleanRecord(tableSchema, record);
    expect(result).toEqual({ id: 'user:2' });
  });

  it('does not coerce or transform values', () => {
    const date = new Date('2024-01-01');
    const schema = { created: { type: 'string' as const, optional: false, dateTime: true } };
    const record = { id: 'x:1', created: date };
    const result = cleanRecord(schema, record);
    expect(result.created).toBe(date);
  });

  it('returns only id when record has no schema fields', () => {
    const record = { id: 'user:3', unknown1: 'a', unknown2: 'b' };
    const result = cleanRecord(tableSchema, record);
    expect(result).toEqual({ id: 'user:3' });
  });

  it('handles empty record', () => {
    const result = cleanRecord(tableSchema, {});
    expect(result).toEqual({});
  });
});
