import { describe, expect, it } from 'vitest';
import {
  emptyBackendInfo,
  emptyBackendVersions,
  parseBackendInfo,
  toEntityArray,
  UNAVAILABLE,
} from './versions';

describe('toEntityArray', () => {
  it('passes through an array, dropping non-objects', () => {
    expect(toEntityArray([{ entity: 'ssp' }, null, 'x'])).toEqual([{ entity: 'ssp' }]);
  });

  it('wraps a single object', () => {
    expect(toEntityArray({ entity: 'ssp' })).toEqual([{ entity: 'ssp' }]);
  });

  it('returns [] for null/undefined/primitives', () => {
    expect(toEntityArray(null)).toEqual([]);
    expect(toEntityArray(undefined)).toEqual([]);
    expect(toEntityArray(42)).toEqual([]);
  });
});

describe('parseBackendInfo', () => {
  it('parses the singlenode shape (ssp only) incl. surrealdb_version', () => {
    const { versions, entities } = parseBackendInfo([
      { entity: 'ssp', version: '0.0.1-canary.69', surrealdb_version: '2.0.3', status: 'ready' },
    ]);
    expect(versions.ssp).toBe('0.0.1-canary.69');
    expect(versions.surrealdb).toBe('2.0.3');
    expect(versions.scheduler).toBe(UNAVAILABLE);
    expect(entities).toHaveLength(1);
  });

  it('parses the cluster shape (scheduler + ssp + backend)', () => {
    const { versions, entities } = parseBackendInfo([
      { entity: 'scheduler', version: '0.9.0', surrealdb_version: '2.0.3', status: 'ready' },
      { entity: 'ssp', version: '0.9.0', status: 'ready' },
      { entity: 'backend', id: 'surrealdb', status: 'healthy' },
    ]);
    expect(versions.scheduler).toBe('0.9.0');
    expect(versions.ssp).toBe('0.9.0');
    expect(versions.surrealdb).toBe('2.0.3');
    expect(entities).toHaveLength(3);
  });

  it('strips a leading surrealdb- prefix', () => {
    const { versions } = parseBackendInfo([
      { entity: 'ssp', version: '1.0.0', surrealdb_version: 'surrealdb-2.1.0' },
    ]);
    expect(versions.surrealdb).toBe('2.1.0');
  });

  it('takes surrealdb_version from whichever entity reports it', () => {
    const { versions } = parseBackendInfo([
      { entity: 'scheduler', version: '0.9.0' },
      { entity: 'ssp', version: '0.9.0', surrealdb_version: '3.1.0' },
    ]);
    expect(versions.surrealdb).toBe('3.1.0');
  });

  it('tolerates a single object instead of an array', () => {
    const { versions } = parseBackendInfo({ entity: 'ssp', version: '1.2.3' });
    expect(versions.ssp).toBe('1.2.3');
  });

  it('returns all-unavailable / empty for null or garbage', () => {
    expect(parseBackendInfo(null)).toEqual(emptyBackendInfo());
    expect(parseBackendInfo('nope').versions).toEqual(emptyBackendVersions());
    expect(parseBackendInfo([{ foo: 'bar' }]).versions).toEqual(emptyBackendVersions());
  });
});
