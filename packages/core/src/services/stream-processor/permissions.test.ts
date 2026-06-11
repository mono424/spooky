import { describe, it, expect } from 'vitest';
import { extractSelectPermissions } from './permissions';

describe('extractSelectPermissions', () => {
  it('maps a permissive multi-action table to true', () => {
    const surql = `DEFINE TABLE game SCHEMAFULL
      PERMISSIONS FOR select, create, update, delete WHERE true;`;
    expect(extractSelectPermissions(surql)).toEqual({ game: 'true' });
  });

  it('handles FULL and NONE', () => {
    const surql = `
      DEFINE TABLE a PERMISSIONS FULL;
      DEFINE TABLE b PERMISSIONS NONE;`;
    expect(extractSelectPermissions(surql)).toEqual({ a: 'true', b: 'false' });
  });

  it('extracts the select predicate from multiple FOR groups', () => {
    const surql = `DEFINE TABLE doc SCHEMAFULL
      PERMISSIONS
        FOR create, update, delete WHERE owner = $auth.id
        FOR select WHERE owner = $auth.id OR public = true;`;
    expect(extractSelectPermissions(surql)).toEqual({
      doc: 'owner = $auth.id OR public = true',
    });
  });

  it('returns false when permissions exist but none grant select', () => {
    const surql = `DEFINE TABLE log PERMISSIONS FOR create WHERE true;`;
    expect(extractSelectPermissions(surql)).toEqual({ log: 'false' });
  });

  it('treats a missing PERMISSIONS clause as default-deny', () => {
    const surql = `DEFINE TABLE bare SCHEMALESS;`;
    expect(extractSelectPermissions(surql)).toEqual({ bare: 'false' });
  });

  it('ignores commented-out clauses and handles OVERWRITE / IF NOT EXISTS', () => {
    const surql = `
      -- DEFINE TABLE ghost PERMISSIONS FULL;
      DEFINE TABLE OVERWRITE u PERMISSIONS FOR select WHERE true;
      DEFINE TABLE IF NOT EXISTS v PERMISSIONS FULL;`;
    const perms = extractSelectPermissions(surql);
    expect(perms).toEqual({ u: 'true', v: 'true' });
    expect(perms.ghost).toBeUndefined();
  });
});
