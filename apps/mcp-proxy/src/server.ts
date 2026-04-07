import { McpServer, ResourceTemplate } from '@modelcontextprotocol/sdk/server/mcp.js';
import { z } from 'zod';
import { SurrealClient } from './surreal.js';

export function createServer(surreal: SurrealClient): McpServer {
  const server = new McpServer({
    name: 'sp00ky-mcp-proxy',
    version: '0.0.1',
  });

  // --- Tools ---

  server.tool(
    'run_query',
    'Execute a SurrealQL query against the database',
    {
      query: z.string().describe('SurrealQL query to execute'),
    },
    async ({ query }) => {
      const result = await surreal.query(query);
      return { content: [{ type: 'text' as const, text: JSON.stringify(result, null, 2) }] };
    }
  );

  server.tool(
    'list_tables',
    'List all database tables',
    {},
    async () => {
      const result = await surreal.query('INFO FOR DB;');
      const info = result as any[];
      const tables = info?.[0]?.result?.tables ?? info?.[0]?.tables ?? {};
      const tableNames = Object.keys(tables);
      return { content: [{ type: 'text' as const, text: JSON.stringify(tableNames, null, 2) }] };
    }
  );

  server.tool(
    'get_table_data',
    'Fetch records from a database table',
    {
      tableName: z.string().describe('Name of the table'),
      limit: z.number().optional().default(100).describe('Max number of records to return'),
    },
    async ({ tableName, limit }) => {
      const result = await surreal.query(`SELECT * FROM \`${tableName}\` LIMIT ${limit};`);
      return { content: [{ type: 'text' as const, text: JSON.stringify(result, null, 2) }] };
    }
  );

  server.tool(
    'update_table_row',
    'Update a record in a database table',
    {
      recordId: z.string().describe('Record ID to update (e.g. "users:abc123")'),
      updates: z.record(z.unknown()).describe('Fields to update'),
    },
    async ({ recordId, updates }) => {
      const result = await surreal.query(
        `UPDATE ${recordId} MERGE ${JSON.stringify(updates)};`
      );
      return { content: [{ type: 'text' as const, text: JSON.stringify(result, null, 2) }] };
    }
  );

  server.tool(
    'delete_table_row',
    'Delete a record from a database table',
    {
      recordId: z.string().describe('Record ID to delete (e.g. "users:abc123")'),
    },
    async ({ recordId }) => {
      const result = await surreal.query(`DELETE ${recordId};`);
      return { content: [{ type: 'text' as const, text: JSON.stringify(result, null, 2) }] };
    }
  );

  server.tool(
    'get_active_queries',
    'Get all active live queries registered with the SSP',
    {},
    async () => {
      const result = await surreal.query('SELECT * FROM _00_query;');
      return { content: [{ type: 'text' as const, text: JSON.stringify(result, null, 2) }] };
    }
  );

  server.tool(
    'get_events',
    'Get event history, optionally limited',
    {
      limit: z.number().optional().default(50).describe('Max number of events to return'),
    },
    async ({ limit }) => {
      const result = await surreal.query(
        `SELECT * FROM _00_events ORDER BY timestamp DESC LIMIT ${limit};`
      );
      return { content: [{ type: 'text' as const, text: JSON.stringify(result, null, 2) }] };
    }
  );

  // --- Resources ---

  server.resource('tables', 'sp00ky://tables', { description: 'List of database tables' }, async (uri) => {
    const result = await surreal.query('INFO FOR DB;');
    const info = result as any[];
    const tables = info?.[0]?.result?.tables ?? info?.[0]?.tables ?? {};
    const tableNames = Object.keys(tables);
    return { contents: [{ uri: uri.href, mimeType: 'application/json', text: JSON.stringify(tableNames, null, 2) }] };
  });

  server.resource(
    'table-data',
    new ResourceTemplate('sp00ky://tables/{tableName}', { list: undefined }),
    { description: 'Contents of a specific database table' },
    async (uri, variables) => {
      const tableName = variables.tableName as string;
      const result = await surreal.query(`SELECT * FROM \`${tableName}\` LIMIT 100;`);
      return { contents: [{ uri: uri.href, mimeType: 'application/json', text: JSON.stringify(result, null, 2) }] };
    }
  );

  server.resource('queries', 'sp00ky://queries', { description: 'Active live queries' }, async (uri) => {
    const result = await surreal.query('SELECT * FROM _00_query;');
    return { contents: [{ uri: uri.href, mimeType: 'application/json', text: JSON.stringify(result, null, 2) }] };
  });

  server.resource('events', 'sp00ky://events', { description: 'Event history' }, async (uri) => {
    const result = await surreal.query('SELECT * FROM _00_events ORDER BY timestamp DESC LIMIT 50;');
    return { contents: [{ uri: uri.href, mimeType: 'application/json', text: JSON.stringify(result, null, 2) }] };
  });

  return server;
}
