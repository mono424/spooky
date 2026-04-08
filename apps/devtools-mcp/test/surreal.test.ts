import { describe, it, expect, vi, beforeEach } from 'vitest';
import { SurrealClient } from '../src/surreal.js';

describe('SurrealClient', () => {
  const config = {
    url: 'http://localhost:8666',
    namespace: 'test_ns',
    database: 'test_db',
    username: 'root',
    password: 'root',
  };

  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it('sends correct headers and body', async () => {
    const mockResponse = [{ result: [{ id: 'users:1', name: 'Alice' }] }];
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response(JSON.stringify(mockResponse), { status: 200 })
    );

    const client = new SurrealClient(config);
    const result = await client.query('SELECT * FROM users;');

    expect(fetchSpy).toHaveBeenCalledOnce();
    const [url, opts] = fetchSpy.mock.calls[0];
    expect(url).toBe('http://localhost:8666/sql');
    expect(opts?.method).toBe('POST');
    expect(opts?.body).toBe('SELECT * FROM users;');

    const headers = opts?.headers as Record<string, string>;
    expect(headers['Authorization']).toBe(
      'Basic ' + Buffer.from('root:root').toString('base64')
    );
    expect(headers['surreal-ns']).toBe('test_ns');
    expect(headers['surreal-db']).toBe('test_db');
    expect(headers['Accept']).toBe('application/json');

    expect(result).toEqual(mockResponse);
  });

  it('throws on non-OK response', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response('Unauthorized', { status: 401 })
    );

    const client = new SurrealClient(config);
    await expect(client.query('SELECT 1;')).rejects.toThrow(
      'SurrealDB query failed (401): Unauthorized'
    );
  });
});
