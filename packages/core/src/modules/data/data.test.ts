import { describe, it, expect } from 'vitest';
import { parseUpdateOptions } from './index';

describe('parseUpdateOptions', () => {
  it('returns empty options when no debounce', () => {
    const result = parseUpdateOptions('user:1', { name: 'Alice' });
    expect(result).toEqual({});
  });

  it('returns empty options when options is undefined', () => {
    const result = parseUpdateOptions('user:1', { name: 'Alice' }, undefined);
    expect(result).toEqual({});
  });

  it('returns empty options when debounced is false', () => {
    const result = parseUpdateOptions('user:1', { name: 'Alice' }, { debounced: false });
    expect(result).toEqual({});
  });

  it('debounced: true uses default delay (200) and key = id', () => {
    const result = parseUpdateOptions('user:1', { name: 'Alice' }, { debounced: true });
    expect(result).toEqual({
      debounced: {
        delay: 200,
        key: 'user:1',
      },
    });
  });

  it('supports custom delay', () => {
    const result = parseUpdateOptions('user:1', { name: 'Alice' }, {
      debounced: { delay: 500 },
    });
    expect(result.debounced?.delay).toBe(500);
  });

  it('supports custom key type recordId', () => {
    const result = parseUpdateOptions('user:1', { name: 'Alice', age: 30 }, {
      debounced: { key: 'recordId' },
    });
    expect(result.debounced?.key).toBe('user:1');
  });

  it('key: recordId_x_fields generates composite key from id + sorted field names', () => {
    const result = parseUpdateOptions(
      'user:1',
      { name: 'Alice', age: 30, email: 'a@b.com' },
      { debounced: { key: 'recordId_x_fields' } }
    );
    expect(result.debounced?.key).toBe('user:1::age#email#name');
  });

  it('uses defaults when debounced is object with no key or delay', () => {
    const result = parseUpdateOptions('user:1', { name: 'Alice' }, { debounced: {} });
    expect(result.debounced?.delay).toBe(200);
    expect(result.debounced?.key).toBe('user:1');
  });
});
