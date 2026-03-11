import { McpServer, ResourceTemplate } from '@modelcontextprotocol/sdk/server/mcp.js';
import { z } from 'zod';
import { Bridge } from './bridge.js';
import { BRIDGE_METHODS } from './protocol.js';

export function createServer(bridge: Bridge): McpServer {
  const server = new McpServer({
    name: 'spooky-devtools',
    version: '0.0.1',
  });

  // --- Tools ---

  server.tool(
    'list_connections',
    'List browser tabs connected to Spooky DevTools',
    {},
    async () => {
      const tabs = bridge.getConnectedTabs();
      return {
        content: [
          {
            type: 'text' as const,
            text: JSON.stringify(
              {
                connected: bridge.isConnected,
                tabs,
              },
              null,
              2
            ),
          },
        ],
      };
    }
  );

  server.tool(
    'get_state',
    'Get the full Spooky DevTools state (events, queries, auth, database)',
    { tabId: z.number().optional().describe('Browser tab ID (uses first connected tab if omitted)') },
    async ({ tabId }) => {
      const result = await bridge.request(BRIDGE_METHODS.GET_STATE, {}, tabId);
      return { content: [{ type: 'text' as const, text: JSON.stringify(result, null, 2) }] };
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
      const result = await bridge.request(BRIDGE_METHODS.RUN_QUERY, { query, target }, tabId);
      return { content: [{ type: 'text' as const, text: JSON.stringify(result, null, 2) }] };
    }
  );

  server.tool(
    'list_tables',
    'List all database tables',
    { tabId: z.number().optional().describe('Browser tab ID') },
    async ({ tabId }) => {
      const state = (await bridge.request(BRIDGE_METHODS.GET_STATE, {}, tabId)) as any;
      const tables = state?.database?.tables ?? [];
      return { content: [{ type: 'text' as const, text: JSON.stringify(tables, null, 2) }] };
    }
  );

  server.tool(
    'get_table_data',
    'Fetch all records from a database table',
    {
      tableName: z.string().describe('Name of the table'),
      tabId: z.number().optional().describe('Browser tab ID'),
    },
    async ({ tableName, tabId }) => {
      const result = await bridge.request(BRIDGE_METHODS.GET_TABLE_DATA, { tableName }, tabId);
      return { content: [{ type: 'text' as const, text: JSON.stringify(result, null, 2) }] };
    }
  );

  server.tool(
    'update_table_row',
    'Update a record in a database table',
    {
      tableName: z.string().describe('Name of the table'),
      recordId: z.string().describe('Record ID to update'),
      updates: z.record(z.unknown()).describe('Fields to update'),
      tabId: z.number().optional().describe('Browser tab ID'),
    },
    async ({ tableName, recordId, updates, tabId }) => {
      const result = await bridge.request(
        BRIDGE_METHODS.UPDATE_TABLE_ROW,
        { tableName, recordId, updates },
        tabId
      );
      return { content: [{ type: 'text' as const, text: JSON.stringify(result, null, 2) }] };
    }
  );

  server.tool(
    'delete_table_row',
    'Delete a record from a database table',
    {
      tableName: z.string().describe('Name of the table'),
      recordId: z.string().describe('Record ID to delete'),
      tabId: z.number().optional().describe('Browser tab ID'),
    },
    async ({ tableName, recordId, tabId }) => {
      const result = await bridge.request(
        BRIDGE_METHODS.DELETE_TABLE_ROW,
        { tableName, recordId },
        tabId
      );
      return { content: [{ type: 'text' as const, text: JSON.stringify(result, null, 2) }] };
    }
  );

  server.tool(
    'get_active_queries',
    'Get all active live queries and their data',
    { tabId: z.number().optional().describe('Browser tab ID') },
    async ({ tabId }) => {
      const state = (await bridge.request(BRIDGE_METHODS.GET_STATE, {}, tabId)) as any;
      const queries = state?.activeQueries ?? [];
      return { content: [{ type: 'text' as const, text: JSON.stringify(queries, null, 2) }] };
    }
  );

  server.tool(
    'get_events',
    'Get event history, optionally filtered by type',
    {
      eventType: z.string().optional().describe('Filter by event type'),
      limit: z.number().optional().describe('Max number of events to return'),
      tabId: z.number().optional().describe('Browser tab ID'),
    },
    async ({ eventType, limit, tabId }) => {
      const state = (await bridge.request(BRIDGE_METHODS.GET_STATE, {}, tabId)) as any;
      let events = state?.eventsHistory ?? [];
      if (eventType) {
        events = events.filter((e: any) => e.eventType === eventType);
      }
      if (limit) {
        events = events.slice(-limit);
      }
      return { content: [{ type: 'text' as const, text: JSON.stringify(events, null, 2) }] };
    }
  );

  server.tool(
    'get_auth_state',
    'Get the current authentication state',
    { tabId: z.number().optional().describe('Browser tab ID') },
    async ({ tabId }) => {
      const state = (await bridge.request(BRIDGE_METHODS.GET_STATE, {}, tabId)) as any;
      const auth = state?.auth ?? null;
      return { content: [{ type: 'text' as const, text: JSON.stringify(auth, null, 2) }] };
    }
  );

  server.tool(
    'clear_history',
    'Clear the event history',
    { tabId: z.number().optional().describe('Browser tab ID') },
    async ({ tabId }) => {
      await bridge.request(BRIDGE_METHODS.CLEAR_HISTORY, {}, tabId);
      return { content: [{ type: 'text' as const, text: 'History cleared.' }] };
    }
  );

  // --- Resources ---

  server.resource('state', 'spooky://state', { description: 'Full Spooky DevTools state' }, async (uri) => {
    const state = await bridge.request(BRIDGE_METHODS.GET_STATE);
    return { contents: [{ uri: uri.href, mimeType: 'application/json', text: JSON.stringify(state, null, 2) }] };
  });

  server.resource('tables', 'spooky://tables', { description: 'List of database tables' }, async (uri) => {
    const state = (await bridge.request(BRIDGE_METHODS.GET_STATE)) as any;
    const tables = state?.database?.tables ?? [];
    return { contents: [{ uri: uri.href, mimeType: 'application/json', text: JSON.stringify(tables, null, 2) }] };
  });

  server.resource(
    'table-data',
    new ResourceTemplate('spooky://tables/{tableName}', { list: undefined }),
    { description: 'Contents of a specific database table' },
    async (uri, variables) => {
      const tableName = variables.tableName as string;
      const result = await bridge.request(BRIDGE_METHODS.GET_TABLE_DATA, { tableName });
      return { contents: [{ uri: uri.href, mimeType: 'application/json', text: JSON.stringify(result, null, 2) }] };
    }
  );

  server.resource('queries', 'spooky://queries', { description: 'Active live queries' }, async (uri) => {
    const state = (await bridge.request(BRIDGE_METHODS.GET_STATE)) as any;
    const queries = state?.activeQueries ?? [];
    return { contents: [{ uri: uri.href, mimeType: 'application/json', text: JSON.stringify(queries, null, 2) }] };
  });

  server.resource('events', 'spooky://events', { description: 'Event history' }, async (uri) => {
    const state = (await bridge.request(BRIDGE_METHODS.GET_STATE)) as any;
    const events = state?.eventsHistory ?? [];
    return { contents: [{ uri: uri.href, mimeType: 'application/json', text: JSON.stringify(events, null, 2) }] };
  });

  return server;
}
