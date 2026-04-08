import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import type { EventDefinition } from './index';
import { EventSystem } from './index';

// Define test event types
type TestEvents = {
  userCreated: EventDefinition<'userCreated', { id: string; name: string }>;
  userUpdated: EventDefinition<'userUpdated', { id: string; changes: string[] }>;
  ping: EventDefinition<'ping', undefined>;
};

function createTestSystem(): EventSystem<TestEvents> {
  return new EventSystem<TestEvents>(['userCreated', 'userUpdated', 'ping']);
}

/** Flush microtasks so queueMicrotask-scheduled processing runs. */
async function flush() {
  await new Promise<void>((resolve) => queueMicrotask(resolve));
}

describe('EventSystem', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  describe('subscribe + emit', () => {
    it('handler receives event with correct type and payload', async () => {
      const system = createTestSystem();
      const handler = vi.fn();

      system.subscribe('userCreated', handler);
      system.emit('userCreated', { id: '1', name: 'Alice' });

      await flush();

      expect(handler).toHaveBeenCalledOnce();
      expect(handler).toHaveBeenCalledWith({
        type: 'userCreated',
        payload: { id: '1', name: 'Alice' },
      });
    });

    it('does not call handler for different event type', async () => {
      const system = createTestSystem();
      const handler = vi.fn();

      system.subscribe('userCreated', handler);
      system.emit('userUpdated', { id: '1', changes: ['name'] });

      await flush();

      expect(handler).not.toHaveBeenCalled();
    });
  });

  describe('multiple subscribers', () => {
    it('all handlers are called', async () => {
      const system = createTestSystem();
      const handler1 = vi.fn();
      const handler2 = vi.fn();

      system.subscribe('userCreated', handler1);
      system.subscribe('userCreated', handler2);
      system.emit('userCreated', { id: '1', name: 'Alice' });

      await flush();

      expect(handler1).toHaveBeenCalledOnce();
      expect(handler2).toHaveBeenCalledOnce();
    });
  });

  describe('unsubscribe', () => {
    it('handler no longer called after unsubscribe', async () => {
      const system = createTestSystem();
      const handler = vi.fn();

      const id = system.subscribe('userCreated', handler);
      system.unsubscribe(id);
      system.emit('userCreated', { id: '1', name: 'Alice' });

      await flush();

      expect(handler).not.toHaveBeenCalled();
    });

    it('returns true when subscription exists', () => {
      const system = createTestSystem();
      const id = system.subscribe('userCreated', vi.fn());
      expect(system.unsubscribe(id)).toBe(true);
    });

    it('returns false when subscription does not exist', () => {
      const system = createTestSystem();
      expect(system.unsubscribe(999)).toBe(false);
    });
  });

  describe('subscribeMany', () => {
    it('subscribes to multiple event types', async () => {
      const system = createTestSystem();
      const handler = vi.fn();

      const ids = system.subscribeMany(['userCreated', 'userUpdated'], handler);
      expect(ids).toHaveLength(2);

      system.emit('userCreated', { id: '1', name: 'Alice' });
      system.emit('userUpdated', { id: '1', changes: ['name'] });

      await flush();

      expect(handler).toHaveBeenCalledTimes(2);
    });
  });

  describe('once option', () => {
    it('handler called only once then auto-removed', async () => {
      const system = createTestSystem();
      const handler = vi.fn();

      system.subscribe('userCreated', handler, { once: true });

      system.emit('userCreated', { id: '1', name: 'Alice' });
      await flush();

      system.emit('userCreated', { id: '2', name: 'Bob' });
      await flush();

      expect(handler).toHaveBeenCalledOnce();
    });
  });

  describe('immediately option', () => {
    it('handler called with last event on subscribe', async () => {
      const system = createTestSystem();

      // Emit an event first
      system.emit('userCreated', { id: '1', name: 'Alice' });
      await flush();

      // Subscribe with immediately option
      const handler = vi.fn();
      system.subscribe('userCreated', handler, { immediately: true });

      // Handler should be called synchronously with last event
      expect(handler).toHaveBeenCalledOnce();
      expect(handler).toHaveBeenCalledWith({
        type: 'userCreated',
        payload: { id: '1', name: 'Alice' },
      });
    });

    it('does not call handler if no prior event', () => {
      const system = createTestSystem();
      const handler = vi.fn();

      system.subscribe('userCreated', handler, { immediately: true });

      expect(handler).not.toHaveBeenCalled();
    });
  });

  describe('debounced events', () => {
    it('only fires the last event after delay', async () => {
      const system = createTestSystem();
      const handler = vi.fn();

      system.subscribe('userUpdated', handler);

      // Add debounced events rapidly
      system.addEvent(
        { type: 'userUpdated', payload: { id: '1', changes: ['a'] } },
        { debounced: { key: 'update-1', delay: 100 } }
      );
      system.addEvent(
        { type: 'userUpdated', payload: { id: '1', changes: ['b'] } },
        { debounced: { key: 'update-1', delay: 100 } }
      );
      system.addEvent(
        { type: 'userUpdated', payload: { id: '1', changes: ['c'] } },
        { debounced: { key: 'update-1', delay: 100 } }
      );

      // Before delay, nothing should fire
      await flush();
      expect(handler).not.toHaveBeenCalled();

      // After delay
      await vi.advanceTimersByTimeAsync(100);
      await flush();

      expect(handler).toHaveBeenCalledOnce();
      expect(handler).toHaveBeenCalledWith({
        type: 'userUpdated',
        payload: { id: '1', changes: ['c'] },
      });
    });

    it('different keys debounce independently', async () => {
      const system = createTestSystem();
      const handler = vi.fn();

      system.subscribe('userUpdated', handler);

      system.addEvent(
        { type: 'userUpdated', payload: { id: '1', changes: ['a'] } },
        { debounced: { key: 'key-1', delay: 100 } }
      );
      system.addEvent(
        { type: 'userUpdated', payload: { id: '2', changes: ['b'] } },
        { debounced: { key: 'key-2', delay: 100 } }
      );

      await vi.advanceTimersByTimeAsync(100);
      await flush();

      expect(handler).toHaveBeenCalledTimes(2);
    });
  });

  describe('event buffering', () => {
    it('multiple emits in same tick processed in order', async () => {
      const system = createTestSystem();
      const received: string[] = [];

      system.subscribe('userCreated', (event) => {
        received.push(event.payload.name);
      });

      system.emit('userCreated', { id: '1', name: 'Alice' });
      system.emit('userCreated', { id: '2', name: 'Bob' });
      system.emit('userCreated', { id: '3', name: 'Charlie' });

      await flush();

      expect(received).toEqual(['Alice', 'Bob', 'Charlie']);
    });
  });
});
