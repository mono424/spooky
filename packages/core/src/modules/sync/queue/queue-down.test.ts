import { describe, it, expect } from 'vitest';
import { DownQueue } from './queue-down';
import type { DownEvent } from './queue-down';
import type { LocalStore } from '../../../services/database/index';

const silentLogger = {
  child: () => silentLogger,
  debug: () => {},
  info: () => {},
  warn: () => {},
  error: () => {},
} as any;

const register = (hash: string) => ({ type: 'register', payload: { hash } }) as DownEvent;
const cleanup = (hash: string) => ({ type: 'cleanup', payload: { hash } }) as DownEvent;

const hashOf = (e: DownEvent) => e.payload.hash;

function makeQueue() {
  return new DownQueue({} as LocalStore, silentLogger);
}

describe('DownQueue.next failure handling', () => {
  it('re-heads a failing event so a transient failure keeps its order', async () => {
    const q = makeQueue();
    q.push(register('a'));
    q.push(register('b'));

    const seen: string[] = [];
    await expect(
      q.next(async (e) => {
        seen.push(hashOf(e));
        throw new Error('503');
      })
    ).rejects.toThrow('503');

    expect(seen).toEqual(['a']);
    // 'a' is back at the head, ahead of 'b'.
    await expect(q.next(async (e) => { seen.push(hashOf(e)); throw new Error('503'); })).rejects.toThrow();
    expect(seen).toEqual(['a', 'a']);
  });

  it('rotates a persistently failing event to the back so others can proceed', async () => {
    const q = makeQueue();
    q.push(register('poison'));
    q.push(register('healthy'));

    const seen: string[] = [];
    const failPoison = async (e: DownEvent) => {
      seen.push(hashOf(e));
      if (hashOf(e) === 'poison') throw new Error('400 rejected');
    };

    // Three attempts keep the head; the third rotates it behind 'healthy'.
    for (let i = 0; i < 3; i++) {
      await expect(q.next(failPoison)).rejects.toThrow();
    }
    expect(seen).toEqual(['poison', 'poison', 'poison']);

    // 'healthy' is no longer starved.
    await q.next(failPoison);
    expect(seen).toEqual(['poison', 'poison', 'poison', 'healthy']);
    expect(q.size).toBe(1); // only the poison event remains
  });

  it('does not rotate when nothing else is waiting', async () => {
    const q = makeQueue();
    q.push(register('only'));

    for (let i = 0; i < 5; i++) {
      await expect(
        q.next(async () => {
          throw new Error('boom');
        })
      ).rejects.toThrow();
    }
    expect(q.size).toBe(1);
  });

  it('clears the failure count once an event succeeds', async () => {
    const q = makeQueue();
    const event = register('a');
    q.push(event);
    q.push(register('b'));

    await expect(
      q.next(async () => {
        throw new Error('boom');
      })
    ).rejects.toThrow();
    // Succeeds on the retry, so its streak resets rather than carrying over.
    await q.next(async () => {});
    expect(q.size).toBe(1);

    q.push(event);
    // A fresh streak: it keeps the head again rather than rotating immediately.
    await expect(
      q.next(async () => {
        throw new Error('boom');
      })
    ).rejects.toThrow();
    const drained: string[] = [];
    await q.next(async (e) => {
      drained.push(hashOf(e));
    });
    expect(drained).toEqual(['b']);
  });
});

describe('DownQueue.takeNext (per-hash ordering under concurrency)', () => {
  it('hands out events for distinct hashes so they can run in parallel', () => {
    const q = makeQueue();
    q.push(register('a'));
    q.push(register('b'));
    q.push(register('c'));
    const busy = new Set<string>();

    const first = q.takeNext(busy)!;
    busy.add(hashOf(first));
    const second = q.takeNext(busy)!;
    busy.add(hashOf(second));

    expect([hashOf(first), hashOf(second)]).toEqual(['a', 'b']);
    expect(q.size).toBe(1);
  });

  it('SKIPS an event whose hash is busy without reordering it', () => {
    // The whole correctness argument: a `cleanup` must never overtake the
    // `register` for the same query. A busy hash's event keeps its place in the
    // queue and is simply passed over.
    const q = makeQueue();
    q.push(register('a'));
    q.push(cleanup('a'));
    q.push(register('b'));

    const busy = new Set(['a']);
    const taken = q.takeNext(busy)!;
    expect(hashOf(taken)).toBe('b');

    // Once 'a' frees up, its two events come back out in their original order.
    const rest = [q.takeNext(new Set())!, q.takeNext(new Set())!];
    expect(rest.map((e) => e.type)).toEqual(['register', 'cleanup']);
    expect(q.size).toBe(0);
  });

  it('returns undefined when every remaining event is blocked', () => {
    const q = makeQueue();
    q.push(register('a'));
    expect(q.takeNext(new Set(['a']))).toBeUndefined();
    // Blocked, not consumed.
    expect(q.size).toBe(1);
  });
});

describe('DownQueue.run', () => {
  it('returns the error instead of throwing, and re-heads the event', async () => {
    // A concurrent drain has other work in flight when one event fails; a
    // rejection here would take that work down with it.
    const q = makeQueue();
    const event = register('a');
    q.push(event);
    const taken = q.takeNext(new Set())!;

    const boom = new Error('nope');
    const err = await q.run(taken, async () => {
      throw boom;
    });

    expect(err).toBe(boom);
    expect(q.size).toBe(1);
    expect(hashOf(q.takeNext(new Set())!)).toBe('a');
  });

  it('returns undefined on success', async () => {
    const q = makeQueue();
    q.push(register('a'));
    const taken = q.takeNext(new Set())!;
    await expect(q.run(taken, async () => {})).resolves.toBeUndefined();
    expect(q.size).toBe(0);
  });
});

describe('DownQueue.push register coalescing', () => {
  it('drops a register for a hash that already has one queued', () => {
    // Four paths re-enqueue `register` for every active hash (reconnect,
    // self-heal, re-mount, and a re-headed failure) and they stack: one tab was
    // measured issuing 84 registrations in 93 seconds for 38 hashes. A register
    // carries nothing but the hash, so the second is identical work.
    const q = makeQueue();
    q.push(register('a'));
    q.push(register('a'));
    q.push(register('a'));

    expect(q.size).toBe(1);
  });

  it('keeps registers for DIFFERENT hashes', () => {
    const q = makeQueue();
    q.push(register('a'));
    q.push(register('b'));

    expect(q.size).toBe(2);
  });

  it('does NOT coalesce a register queued behind a cleanup for the same hash', () => {
    // Ordering is per hash: dropping this register would let the cleanup tear
    // the query down with nothing left to re-establish it.
    const q = makeQueue();
    q.push(register('a'));
    q.push(cleanup('a'));
    q.push(register('a'));

    expect(q.size).toBe(3);
    expect([q.takeNext(new Set())!, q.takeNext(new Set())!, q.takeNext(new Set())!].map((e) => e.type)).toEqual([
      'register',
      'cleanup',
      'register',
    ]);
  });

  it('leaves non-register events alone', () => {
    const q = makeQueue();
    q.push(cleanup('a'));
    q.push(cleanup('a'));

    expect(q.size).toBe(2);
  });
});
