import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createServer } from '../src/server.js';
import type { Bridge } from '../src/bridge.js';
import type { SurrealClient } from '../src/surreal.js';

function mockBridge(connected: boolean): Bridge {
  return {
    isConnected: connected,
    getConnectedTabs: vi.fn().mockReturnValue(
      connected ? [{ tabId: 1, url: 'http://localhost', title: 'Test' }] : []
    ),
    request: vi.fn().mockResolvedValue({ mock: 'bridge-response' }),
    start: vi.fn().mockResolvedValue(undefined),
    stop: vi.fn().mockResolvedValue(undefined),
  } as unknown as Bridge;
}

function mockSurreal(): SurrealClient {
  return {
    query: vi.fn().mockResolvedValue([{ result: 'surreal-response' }]),
  } as unknown as SurrealClient;
}

// Helper to call an MCP tool by name via McpServer internals
async function callTool(server: ReturnType<typeof createServer>, name: string, args: Record<string, unknown> = {}) {
  const tools = (server as any)._registeredTools as Record<string, any>;
  const tool = tools[name];
  if (!tool) {
    throw new Error(`Tool "${name}" not found. Available: ${Object.keys(tools).join(', ')}`);
  }
  return tool.handler(args, {} as any);
}

describe('createServer', () => {
  describe('with bridge connected', () => {
    it('run_query uses bridge', async () => {
      const bridge = mockBridge(true);
      const surreal = mockSurreal();
      const server = createServer(bridge, surreal);

      const result = await callTool(server, 'run_query', { query: 'SELECT 1;', target: 'remote' });

      expect((bridge.request as any)).toHaveBeenCalled();
      expect((surreal.query as any)).not.toHaveBeenCalled();
      expect(result.content[0].text).toContain('bridge-response');
    });

    it('list_connections returns tabs', async () => {
      const bridge = mockBridge(true);
      const server = createServer(bridge);

      const result = await callTool(server, 'list_connections');
      const data = JSON.parse(result.content[0].text);

      expect(data.connected).toBe(true);
      expect(data.tabs).toHaveLength(1);
    });
  });

  describe('with bridge disconnected, surreal available', () => {
    it('run_query falls back to surreal', async () => {
      const bridge = mockBridge(false);
      const surreal = mockSurreal();
      const server = createServer(bridge, surreal);

      const result = await callTool(server, 'run_query', { query: 'SELECT 1;', target: 'remote' });

      expect((bridge.request as any)).not.toHaveBeenCalled();
      expect((surreal.query as any)).toHaveBeenCalledWith('SELECT 1;');
      expect(result.content[0].text).toContain('surreal-response');
    });

    it('list_tables falls back to surreal', async () => {
      const bridge = mockBridge(false);
      const surreal = mockSurreal();
      (surreal.query as any).mockResolvedValue([{ tables: { users: '', posts: '' } }]);
      const server = createServer(bridge, surreal);

      const result = await callTool(server, 'list_tables', {});
      const tables = JSON.parse(result.content[0].text);

      expect(tables).toEqual(['users', 'posts']);
    });

    it('get_table_data falls back to surreal with limit', async () => {
      const bridge = mockBridge(false);
      const surreal = mockSurreal();
      const server = createServer(bridge, surreal);

      await callTool(server, 'get_table_data', { tableName: 'users', limit: 50 });

      expect((surreal.query as any)).toHaveBeenCalledWith('SELECT * FROM `users` LIMIT 50;');
    });
  });

  describe('with bridge disconnected, no surreal', () => {
    it('run_query throws descriptive error', async () => {
      const bridge = mockBridge(false);
      const server = createServer(bridge, null);

      await expect(callTool(server, 'run_query', { query: 'SELECT 1;', target: 'remote' })).rejects.toThrow(
        'No extension connected and no direct database configured'
      );
    });

    it('list_connections returns empty (no error)', async () => {
      const bridge = mockBridge(false);
      const server = createServer(bridge);

      const result = await callTool(server, 'list_connections');
      const data = JSON.parse(result.content[0].text);

      expect(data.connected).toBe(false);
      expect(data.tabs).toEqual([]);
    });

    it('get_state throws (bridge-only)', async () => {
      const bridge = mockBridge(false);
      const server = createServer(bridge, mockSurreal());

      await expect(callTool(server, 'get_state', {})).rejects.toThrow(
        'requires the Sp00ky DevTools browser extension'
      );
    });

    it('get_auth_state throws (bridge-only)', async () => {
      const bridge = mockBridge(false);
      const server = createServer(bridge, mockSurreal());

      await expect(callTool(server, 'get_auth_state', {})).rejects.toThrow(
        'requires the Sp00ky DevTools browser extension'
      );
    });

    it('clear_history throws (bridge-only)', async () => {
      const bridge = mockBridge(false);
      const server = createServer(bridge, mockSurreal());

      await expect(callTool(server, 'clear_history', {})).rejects.toThrow(
        'requires the Sp00ky DevTools browser extension'
      );
    });
  });
});
