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
