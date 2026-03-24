import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { withRetry } from './index';

function createMockLogger() {
  return {
    warn: vi.fn(),
    info: vi.fn(),
    debug: vi.fn(),
    error: vi.fn(),
    fatal: vi.fn(),
    trace: vi.fn(),
    child: vi.fn(),
  } as any;
}

describe('withRetry', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('succeeds on first try and returns result', async () => {
    const logger = createMockLogger();
    const operation = vi.fn().mockResolvedValue('ok');

    const result = await withRetry(logger, operation);

    expect(result).toBe('ok');
    expect(operation).toHaveBeenCalledOnce();
    expect(logger.warn).not.toHaveBeenCalled();
  });

  it('retries on transaction conflict and returns result on success', async () => {
    const logger = createMockLogger();
    const operation = vi
      .fn()
      .mockRejectedValueOnce(new Error('Can not open transaction'))
      .mockResolvedValueOnce('ok');

    const promise = withRetry(logger, operation, 3, 100);

    // First call fails, triggers setTimeout(100 * 1)
    await vi.advanceTimersByTimeAsync(100);

    const result = await promise;
    expect(result).toBe('ok');
    expect(operation).toHaveBeenCalledTimes(2);
    expect(logger.warn).toHaveBeenCalledOnce();
  });

  it('retries on "Database is busy" error', async () => {
    const logger = createMockLogger();
    const operation = vi
      .fn()
      .mockRejectedValueOnce(new Error('Database is busy'))
      .mockResolvedValueOnce('ok');

    const promise = withRetry(logger, operation, 3, 100);
    await vi.advanceTimersByTimeAsync(100);

    const result = await promise;
    expect(result).toBe('ok');
    expect(operation).toHaveBeenCalledTimes(2);
  });

  it('retries on generic "transaction" error', async () => {
    const logger = createMockLogger();
    const operation = vi
      .fn()
      .mockRejectedValueOnce(new Error('Some transaction error'))
      .mockResolvedValueOnce('ok');

    const promise = withRetry(logger, operation, 3, 100);
    await vi.advanceTimersByTimeAsync(100);

    const result = await promise;
    expect(result).toBe('ok');
  });

  it('throws immediately on non-retryable error', async () => {
    const logger = createMockLogger();
    const operation = vi.fn().mockRejectedValue(new Error('Not found'));

    await expect(withRetry(logger, operation)).rejects.toThrow('Not found');
    expect(operation).toHaveBeenCalledOnce();
    expect(logger.warn).not.toHaveBeenCalled();
  });

  it('exhausts all retries and throws last error', async () => {
    // Use real timers for this test to avoid unhandled rejection timing issues
    vi.useRealTimers();
    const logger = createMockLogger();
    const operation = vi
      .fn()
      .mockRejectedValueOnce(new Error('Can not open transaction'))
      .mockRejectedValueOnce(new Error('Can not open transaction'))
      .mockRejectedValueOnce(new Error('Can not open transaction'));

    await expect(withRetry(logger, operation, 3, 1)).rejects.toThrow(
      'Can not open transaction'
    );
    expect(operation).toHaveBeenCalledTimes(3);
    expect(logger.warn).toHaveBeenCalledTimes(3);
  });

  it('uses exponential backoff (delay * attempt)', async () => {
    const logger = createMockLogger();
    let callCount = 0;
    const operation = vi.fn().mockImplementation(() => {
      callCount++;
      if (callCount < 3) {
        return Promise.reject(new Error('transaction conflict'));
      }
      return Promise.resolve('ok');
    });

    const promise = withRetry(logger, operation, 3, 100);

    // First retry: delay = 100 * 1 = 100
    await vi.advanceTimersByTimeAsync(100);
    // Second retry: delay = 100 * 2 = 200
    await vi.advanceTimersByTimeAsync(200);

    const result = await promise;
    expect(result).toBe('ok');
    expect(operation).toHaveBeenCalledTimes(3);
  });

  it('logger.warn is called with retry details', async () => {
    const logger = createMockLogger();
    const operation = vi
      .fn()
      .mockRejectedValueOnce(new Error('Can not open transaction'))
      .mockResolvedValueOnce('ok');

    const promise = withRetry(logger, operation, 3, 100);
    await vi.advanceTimersByTimeAsync(100);
    await promise;

    expect(logger.warn).toHaveBeenCalledWith(
      expect.objectContaining({
        attempt: 1,
        retries: 3,
        error: 'Can not open transaction',
        Category: 'sp00ky-client::utils::withRetry',
      }),
      'Retrying DB operation'
    );
  });
});
