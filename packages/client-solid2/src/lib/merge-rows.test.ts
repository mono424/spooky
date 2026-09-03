import { describe, expect, it } from 'vitest';
import { mergeRows, sameValue } from './merge-rows';

// A stand-in for the SDK's RecordId: a class with a value-carrying toString,
// where two instances of the same record are never `===`.
class Rid {
  constructor(private tb: string, private id: string) {}
  toString() {
    return `${this.tb}:${this.id}`;
  }
}

// Counts field writes on a row, the way a store proxy would notify.
function tracked(row: Record<string, unknown>) {
  const writes: string[] = [];
  const proxy = new Proxy(row, {
    set(target, prop, value) {
      writes.push(String(prop));
      (target as any)[prop] = value;
      return true;
    },
    deleteProperty(target, prop) {
      writes.push(`delete:${String(prop)}`);
      delete (target as any)[prop];
      return true;
    },
  });
  return { proxy, writes };
}

describe('sameValue', () => {
  it('treats two RecordId-like wrappers of the same record as equal', () => {
    expect(sameValue(new Rid('user', 'a'), new Rid('user', 'a'))).toBe(true);
    expect(sameValue(new Rid('user', 'a'), new Rid('user', 'b'))).toBe(false);
  });
  it('compares dates by time', () => {
    expect(sameValue(new Date(5), new Date(5))).toBe(true);
    expect(sameValue(new Date(5), new Date(6))).toBe(false);
  });
  it('keeps identity semantics for plain objects and arrays', () => {
    expect(sameValue({ a: 1 }, { a: 1 })).toBe(false);
    expect(sameValue([1], [1])).toBe(false);
    const o = { a: 1 };
    expect(sameValue(o, o)).toBe(true);
  });
  it('never equates a value with null', () => {
    expect(sameValue(null, new Rid('x', 'y'))).toBe(false);
    expect(sameValue(undefined, null)).toBe(false);
  });
});

describe('mergeRows', () => {
  it('writes nothing when a re-emitted row only differs by wrapper identity', () => {
    const { proxy, writes } = tracked({
      id: new Rid('conversation', 'c1'),
      user_a: new Rid('user', 'a'),
      last_message_ms: 10,
      created_at: new Date(1000),
    });
    const draft = [proxy];
    mergeRows(draft, [
      {
        id: new Rid('conversation', 'c1'),
        user_a: new Rid('user', 'a'),
        last_message_ms: 10,
        created_at: new Date(1000),
      },
    ]);
    expect(writes).toEqual([]);
  });

  it('writes only the field that actually changed', () => {
    const { proxy, writes } = tracked({
      id: new Rid('conversation', 'c1'),
      last_message_ms: 10,
    });
    const draft = [proxy];
    mergeRows(draft, [{ id: new Rid('conversation', 'c1'), last_message_ms: 11 }]);
    expect(writes).toEqual(['last_message_ms']);
    expect((proxy as any).last_message_ms).toBe(11);
  });

  it('still inserts, drops and reorders rows by key', () => {
    const a = { id: 'a', v: 1 };
    const b = { id: 'b', v: 1 };
    const draft: any[] = [a, b];
    mergeRows(draft, [{ id: 'b', v: 2 }, { id: 'c', v: 1 }]);
    expect(draft.map((r) => r.id)).toEqual(['b', 'c']);
    expect(draft[0]).toBe(b);
    expect(b.v).toBe(2);
  });
});
