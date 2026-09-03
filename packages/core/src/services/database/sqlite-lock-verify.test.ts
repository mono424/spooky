import { describe, it, expect, vi } from 'vitest';
import { verifyLockStillHeld } from './sqlite-lock-verify';

describe('verifyLockStillHeld', () => {
  it('fences when the lock is no longer held', async () => {
    const fence = vi.fn(async () => {});
    await verifyLockStillHeld({ query: async () => ({ held: [] }) }, 'lock-a', fence, 50);
    expect(fence).toHaveBeenCalledWith('lock missing after suspected freeze');
  });

  it('does nothing when the lock is still held', async () => {
    const fence = vi.fn(async () => {});
    await verifyLockStillHeld({ query: async () => ({ held: [{ name: 'lock-a' }] }) }, 'lock-a', fence, 50);
    expect(fence).not.toHaveBeenCalled();
  });

  it('resolves within the bound when the lock query never answers, without fencing', async () => {
    const fence = vi.fn(async () => {});
    const warn = vi.fn();
    const started = Date.now();
    await verifyLockStillHeld({ query: () => new Promise(() => {}) }, 'lock-a', fence, 30, warn);
    expect(Date.now() - started).toBeLessThan(1_000);
    expect(fence).not.toHaveBeenCalled();
    expect(warn).toHaveBeenCalledTimes(1);
  });

  it('is a no-op without a lock or a query API', async () => {
    const fence = vi.fn(async () => {});
    await verifyLockStillHeld(null, 'x', fence);
    await verifyLockStillHeld({ query: async () => ({}) }, undefined, fence);
    expect(fence).not.toHaveBeenCalled();
  });
});
