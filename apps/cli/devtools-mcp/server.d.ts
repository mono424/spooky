import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import type { Bridge } from './bridge.js';
import type { SurrealClient } from './surreal.js';
export declare function createServer(bridge: Bridge, surreal?: SurrealClient | null): McpServer;
