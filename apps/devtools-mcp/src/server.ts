import { McpServer, ResourceTemplate } from '@modelcontextprotocol/sdk/server/mcp.js';
import { z } from 'zod';
import type { Bridge } from './bridge.js';
import type { SurrealClient } from './surreal.js';
import { BRIDGE_METHODS } from './protocol.js';

function json(data: unknown) {
  return { content: [{ type: 'text' as const, text: JSON.stringify(data, null, 2) }] };
}

export function createServer(bridge: Bridge, surreal?: SurrealClient | null): McpServer {
  const server = new McpServer({
    name: 'sp00ky-devtools',
    version: '0.0.1',
  });

  // --- Tools ---

  server.tool(
    'list_connections',
    'List browser tabs connected to Sp00ky DevTools',
    {},
    async () => {
      return json({ connected: bridge.isConnected, tabs: bridge.getConnectedTabs() });
    }
  );

  server.tool(
    'get_state',
    'Get the full Sp00ky DevTools state (events, queries, auth, database)',
    { tabId: z.number().optional().describe('Browser tab ID (uses first connected tab if omitted)') },
    async ({ tabId }) => {
      if (!bridge.isConnected) {
        throw new Error('No extension connected. get_state requires the Sp00ky DevTools browser extension.');
      }
      const result = await bridge.request(BRIDGE_METHODS.GET_STATE, {}, tabId);
      return json(result);
    }
  );

  server.tool(
    'run_query',
    'Execute a SurrealQL query against the database',
    {
      query: z.string().describe('SurrealQL query to execute'),
      target: z.enum(['local', 'remote']).optional().default('remote').describe('Query target: local or remote database'),
      tabId: z.number().optional().describe('Browser tab ID'),
    },
    async ({ query, target, tabId }) => {
      if (bridge.isConnected) {
        const result = await bridge.request(BRIDGE_METHODS.RUN_QUERY, { query, target }, tabId);
        return json(result);
      }
      if (surreal) {
        const result = await surreal.query(query);
        return json(result);
      }
      throw new Error('No extension connected and no direct database configured. Set SURREAL_URL or connect the browser extension.');
    }
  );

  server.tool(
    'list_tables',
    'List all database tables',
    { tabId: z.number().optional().describe('Browser tab ID') },
    async ({ tabId }) => {
      if (bridge.isConnected) {
        const state = (await bridge.request(BRIDGE_METHODS.GET_STATE, {}, tabId)) as any;
        const tables = state?.database?.tables ?? [];
        return json(tables);
      }
      if (surreal) {
        const result = await surreal.query('INFO FOR DB;');
        const info = result as any[];
        const tables = info?.[0]?.result?.tables ?? info?.[0]?.tables ?? {};
        return json(Object.keys(tables));
      }
      throw new Error('No extension connected and no direct database configured.');
    }
  );

  server.tool(
    'get_table_data',
    'Fetch records from a database table',
    {
      tableName: z.string().describe('Name of the table'),
      limit: z.number().optional().default(100).describe('Max number of records to return'),
      tabId: z.number().optional().describe('Browser tab ID'),
    },
    async ({ tableName, limit, tabId }) => {
      if (bridge.isConnected) {
        const result = await bridge.request(BRIDGE_METHODS.GET_TABLE_DATA, { tableName }, tabId);
        return json(result);
      }
      if (surreal) {
        const result = await surreal.query(`SELECT * FROM \`${tableName}\` LIMIT ${limit};`);
        return json(result);
      }
      throw new Error('No extension connected and no direct database configured.');
    }
  );

  server.tool(
    'update_table_row',
    'Update a record in a database table',
    {
      tableName: z.string().optional().describe('Name of the table (used when browser extension is connected)'),
      recordId: z.string().describe('Record ID to update (e.g. "users:abc123")'),
      updates: z.record(z.unknown()).describe('Fields to update'),
      tabId: z.number().optional().describe('Browser tab ID'),
    },
    async ({ tableName, recordId, updates, tabId }) => {
      if (bridge.isConnected) {
        const result = await bridge.request(
          BRIDGE_METHODS.UPDATE_TABLE_ROW,
          { tableName, recordId, updates },
          tabId
        );
        return json(result);
      }
      if (surreal) {
        const result = await surreal.query(
          `UPDATE ${recordId} MERGE ${JSON.stringify(updates)};`
        );
        return json(result);
      }
      throw new Error('No extension connected and no direct database configured.');
    }
  );

  server.tool(
    'delete_table_row',
    'Delete a record from a database table',
    {
      tableName: z.string().optional().describe('Name of the table (used when browser extension is connected)'),
      recordId: z.string().describe('Record ID to delete (e.g. "users:abc123")'),
      tabId: z.number().optional().describe('Browser tab ID'),
    },
    async ({ tableName, recordId, tabId }) => {
      if (bridge.isConnected) {
        const result = await bridge.request(
          BRIDGE_METHODS.DELETE_TABLE_ROW,
          { tableName, recordId },
          tabId
        );
        return json(result);
      }
      if (surreal) {
        const result = await surreal.query(`DELETE ${recordId};`);
        return json(result);
      }
      throw new Error('No extension connected and no direct database configured.');
    }
  );

  server.tool(
    'get_active_queries',
    'Get all active live queries and their data',
    { tabId: z.number().optional().describe('Browser tab ID') },
    async ({ tabId }) => {
      if (bridge.isConnected) {
        const state = (await bridge.request(BRIDGE_METHODS.GET_STATE, {}, tabId)) as any;
        return json(state?.activeQueries ?? []);
      }
      if (surreal) {
        const result = await surreal.query('SELECT * FROM _00_query;');
        return json(result);
      }
      throw new Error('No extension connected and no direct database configured.');
    }
  );

  server.tool(
    'get_events',
    'Get event history, optionally filtered by type',
    {
      eventType: z.string().optional().describe('Filter by event type'),
      limit: z.number().optional().default(50).describe('Max number of events to return'),
      tabId: z.number().optional().describe('Browser tab ID'),
    },
    async ({ eventType, limit, tabId }) => {
      if (bridge.isConnected) {
        const state = (await bridge.request(BRIDGE_METHODS.GET_STATE, {}, tabId)) as any;
        let events = state?.eventsHistory ?? [];
        if (eventType) {
          events = events.filter((e: any) => e.eventType === eventType);
        }
        if (limit) {
          events = events.slice(-limit);
        }
        return json(events);
      }
      if (surreal) {
        const result = await surreal.query(
          `SELECT * FROM _00_events ORDER BY timestamp DESC LIMIT ${limit};`
        );
        return json(result);
      }
      throw new Error('No extension connected and no direct database configured.');
    }
  );

  server.tool(
    'get_auth_state',
    'Get the current authentication state',
    { tabId: z.number().optional().describe('Browser tab ID') },
    async ({ tabId }) => {
      if (!bridge.isConnected) {
        throw new Error('No extension connected. get_auth_state requires the Sp00ky DevTools browser extension.');
      }
      const state = (await bridge.request(BRIDGE_METHODS.GET_STATE, {}, tabId)) as any;
      return json(state?.auth ?? null);
    }
  );

  server.tool(
    'clear_history',
    'Clear the event history',
    { tabId: z.number().optional().describe('Browser tab ID') },
    async ({ tabId }) => {
      if (!bridge.isConnected) {
        throw new Error('No extension connected. clear_history requires the Sp00ky DevTools browser extension.');
      }
      await bridge.request(BRIDGE_METHODS.CLEAR_HISTORY, {}, tabId);
      return { content: [{ type: 'text' as const, text: 'History cleared.' }] };
    }
  );

  // --- Resources ---

  server.resource('state', 'sp00ky://state', { description: 'Full Sp00ky DevTools state' }, async (uri) => {
    if (!bridge.isConnected) {
      throw new Error('No extension connected. State resource requires the browser extension.');
    }
    const state = await bridge.request(BRIDGE_METHODS.GET_STATE);
    return { contents: [{ uri: uri.href, mimeType: 'application/json', text: JSON.stringify(state, null, 2) }] };
  });

  server.resource('tables', 'sp00ky://tables', { description: 'List of database tables' }, async (uri) => {
    if (bridge.isConnected) {
      const state = (await bridge.request(BRIDGE_METHODS.GET_STATE)) as any;
      const tables = state?.database?.tables ?? [];
      return { contents: [{ uri: uri.href, mimeType: 'application/json', text: JSON.stringify(tables, null, 2) }] };
    }
    if (surreal) {
      const result = await surreal.query('INFO FOR DB;');
      const info = result as any[];
      const tables = info?.[0]?.result?.tables ?? info?.[0]?.tables ?? {};
      return { contents: [{ uri: uri.href, mimeType: 'application/json', text: JSON.stringify(Object.keys(tables), null, 2) }] };
    }
    throw new Error('No extension connected and no direct database configured.');
  });

  server.resource(
    'table-data',
    new ResourceTemplate('sp00ky://tables/{tableName}', { list: undefined }),
    { description: 'Contents of a specific database table' },
    async (uri, variables) => {
      const tableName = variables.tableName as string;
      if (bridge.isConnected) {
        const result = await bridge.request(BRIDGE_METHODS.GET_TABLE_DATA, { tableName });
        return { contents: [{ uri: uri.href, mimeType: 'application/json', text: JSON.stringify(result, null, 2) }] };
      }
      if (surreal) {
        const result = await surreal.query(`SELECT * FROM \`${tableName}\` LIMIT 100;`);
        return { contents: [{ uri: uri.href, mimeType: 'application/json', text: JSON.stringify(result, null, 2) }] };
      }
      throw new Error('No extension connected and no direct database configured.');
    }
  );

  server.resource('queries', 'sp00ky://queries', { description: 'Active live queries' }, async (uri) => {
    if (bridge.isConnected) {
      const state = (await bridge.request(BRIDGE_METHODS.GET_STATE)) as any;
      const queries = state?.activeQueries ?? [];
      return { contents: [{ uri: uri.href, mimeType: 'application/json', text: JSON.stringify(queries, null, 2) }] };
    }
    if (surreal) {
      const result = await surreal.query('SELECT * FROM _00_query;');
      return { contents: [{ uri: uri.href, mimeType: 'application/json', text: JSON.stringify(result, null, 2) }] };
    }
    throw new Error('No extension connected and no direct database configured.');
  });

  server.resource('events', 'sp00ky://events', { description: 'Event history' }, async (uri) => {
    if (bridge.isConnected) {
      const state = (await bridge.request(BRIDGE_METHODS.GET_STATE)) as any;
      const events = state?.eventsHistory ?? [];
      return { contents: [{ uri: uri.href, mimeType: 'application/json', text: JSON.stringify(events, null, 2) }] };
    }
    if (surreal) {
      const result = await surreal.query('SELECT * FROM _00_events ORDER BY timestamp DESC LIMIT 50;');
      return { contents: [{ uri: uri.href, mimeType: 'application/json', text: JSON.stringify(result, null, 2) }] };
    }
    throw new Error('No extension connected and no direct database configured.');
  });

  return server;
}
