#!/usr/bin/env node

import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { Bridge } from './bridge.js';
import { createServer } from './server.js';

async function main() {
  const bridge = new Bridge();
  await bridge.start();

  const server = createServer(bridge);
  const transport = new StdioServerTransport();
  await server.connect(transport);

  process.stderr.write('[spooky-mcp] MCP server running on stdio\n');

  // Graceful shutdown
  const cleanup = async () => {
    process.stderr.write('[spooky-mcp] Shutting down...\n');
    await bridge.stop();
    process.exit(0);
  };

  process.on('SIGINT', cleanup);
  process.on('SIGTERM', cleanup);
}

main().catch((err) => {
  process.stderr.write(`[spooky-mcp] Fatal error: ${err.message}\n`);
  process.exit(1);
});
