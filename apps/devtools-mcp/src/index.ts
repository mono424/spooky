#!/usr/bin/env node

import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { Bridge } from './bridge.js';
import { SurrealClient } from './surreal.js';
import { createServer } from './server.js';

async function main() {
  const bridge = new Bridge();
  await bridge.start();

  const surreal = process.env.SURREAL_URL
    ? new SurrealClient({
        url: process.env.SURREAL_URL,
        namespace: process.env.SURREAL_NS ?? 'main',
        database: process.env.SURREAL_DB ?? 'main',
        username: process.env.SURREAL_USER ?? 'root',
        password: process.env.SURREAL_PASS ?? 'root',
      })
    : null;

  if (surreal) {
    process.stderr.write(`[sp00ky-mcp] Direct DB mode enabled (${process.env.SURREAL_URL})\n`);
  }

  const server = createServer(bridge, surreal);
  const transport = new StdioServerTransport();
  await server.connect(transport);

  process.stderr.write('[sp00ky-mcp] MCP server running on stdio\n');

  // Graceful shutdown
  const cleanup = async () => {
    process.stderr.write('[sp00ky-mcp] Shutting down...\n');
    await bridge.stop();
    process.exit(0);
  };

  process.on('SIGINT', cleanup);
  process.on('SIGTERM', cleanup);
}

main().catch((err) => {
  process.stderr.write(`[sp00ky-mcp] Fatal error: ${err.message}\n`);
  process.exit(1);
});
