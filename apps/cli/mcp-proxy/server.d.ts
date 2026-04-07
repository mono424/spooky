import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { SurrealClient } from './surreal.js';
export declare function createServer(surreal: SurrealClient): McpServer;
