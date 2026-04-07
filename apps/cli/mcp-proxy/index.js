#!/usr/bin/env node
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { SurrealClient } from './surreal.js';
import { createServer } from './server.js';
function main() {
    const surreal = new SurrealClient({
        url: process.env.SURREAL_URL ?? 'http://localhost:8666',
        namespace: process.env.SURREAL_NS ?? 'main',
        database: process.env.SURREAL_DB ?? 'main',
        username: process.env.SURREAL_USER ?? 'root',
        password: process.env.SURREAL_PASS ?? 'root',
    });
    const server = createServer(surreal);
    const transport = new StdioServerTransport();
    server.connect(transport).then(() => {
        process.stderr.write('[sp00ky-mcp-proxy] MCP server running on stdio\n');
    });
    const cleanup = async () => {
        process.stderr.write('[sp00ky-mcp-proxy] Shutting down...\n');
        process.exit(0);
    };
    process.on('SIGINT', cleanup);
    process.on('SIGTERM', cleanup);
}
main();
