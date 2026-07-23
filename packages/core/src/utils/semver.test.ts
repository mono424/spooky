import { describe, it, expect } from 'vitest';
import { semverGt } from './semver';

describe('semverGt', () => {
  it('compares strictly greater', () => {
    expect(semverGt('1.0.1', '1.0.0')).toBe(true);
    expect(semverGt('1.1.0', '1.0.9')).toBe(true);
    expect(semverGt('2.0.0', '1.9.9')).toBe(true);
    expect(semverGt('1.10.0', '1.9.0')).toBe(true); // numeric, not lexicographic
  });

  it('equal and lower are not greater', () => {
    expect(semverGt('1.0.0', '1.0.0')).toBe(false);
    expect(semverGt('1.0.0', '1.0.1')).toBe(false);
    expect(semverGt('0.9.9', '1.0.0')).toBe(false);
  });

  it('missing parts default to zero', () => {
    expect(semverGt('1.2', '1.2.0')).toBe(false);
    expect(semverGt('1.2.1', '1.2')).toBe(true);
    expect(semverGt('2', '1.9.9')).toBe(true);
  });

  it('junk never compares greater', () => {
    expect(semverGt('abc', '1.0.0')).toBe(false);
    expect(semverGt('1.0.0', 'abc')).toBe(false);
    expect(semverGt('', '0.0.0')).toBe(false);
    expect(semverGt(null, undefined)).toBe(false);
    expect(semverGt('1.0.0-beta', '0.9.0')).toBe(false); // prerelease unsupported
    expect(semverGt('1.2.3.4', '1.0.0')).toBe(false);
  });
});
