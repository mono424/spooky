import { describe, it, expect } from 'vitest';
import { RecordId } from 'surrealdb';
import { listRefTableFor, sanitizeUserId } from './ref-tables';

describe('listRefTableFor', () => {
  it('returns global table in single mode regardless of user', () => {
    expect(listRefTableFor('single', 'user:abc')).toBe('_00_list_ref');
    expect(listRefTableFor('single', null)).toBe('_00_list_ref');
    expect(listRefTableFor('single', undefined)).toBe('_00_list_ref');
  });

  it('returns per-user table in dedicated mode with valid user id', () => {
    expect(listRefTableFor('dedicated', 'user:abc')).toBe(
      '_00_list_ref_user_abc'
    );
  });

  it('accepts a RecordId object in dedicated mode', () => {
    const rid = new RecordId('user', 'def');
    expect(listRefTableFor('dedicated', rid)).toBe('_00_list_ref_user_def');
  });

  it('falls back to global in dedicated mode when user id is missing', () => {
    expect(listRefTableFor('dedicated', null)).toBe('_00_list_ref');
    expect(listRefTableFor('dedicated', undefined)).toBe('_00_list_ref');
  });

  it('falls back to global in dedicated mode when user id has invalid chars', () => {
    // SurrealDB table identifiers only accept alphanumerics + underscore.
    expect(listRefTableFor('dedicated', 'user:abc-with-dash')).toBe(
      '_00_list_ref'
    );
    expect(listRefTableFor('dedicated', 'user:abc.dot')).toBe('_00_list_ref');
  });
});

describe('sanitizeUserId', () => {
  it('strips the user: prefix', () => {
    expect(sanitizeUserId('user:abc123')).toBe('abc123');
  });

  it('accepts plain ids without the user: prefix', () => {
    expect(sanitizeUserId('abc123')).toBe('abc123');
  });

  it('accepts RecordId objects', () => {
    expect(sanitizeUserId(new RecordId('user', 'xyz'))).toBe('xyz');
  });

  it('returns null for invalid id shapes', () => {
    expect(sanitizeUserId(null)).toBeNull();
    expect(sanitizeUserId(undefined)).toBeNull();
    expect(sanitizeUserId('user:')).toBeNull();
    expect(sanitizeUserId('user:has-dash')).toBeNull();
  });
});
