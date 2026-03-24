import { WebSocketServer, WebSocket } from 'ws';
import {
  type BridgeRequest,
  type BridgeResponse,
  type BridgeNotification,
  isBridgeResponse,
  isBridgeNotification,
  BRIDGE_PORT,
} from './protocol.js';

interface ConnectedTab {
  tabId: number;
  url?: string;
  title?: string;
}

interface PendingRequest {
  resolve: (value: unknown) => void;
  reject: (reason: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

const REQUEST_TIMEOUT_MS = 10_000;

export class Bridge {
  private wss: WebSocketServer | null = null;
  private extensionSocket: WebSocket | null = null;
  private connectedTabs: Map<number, ConnectedTab> = new Map();
  private pendingRequests: Map<string, PendingRequest> = new Map();
  private requestCounter = 0;
  private pingInterval: ReturnType<typeof setInterval> | null = null;

  get isConnected(): boolean {
    return this.extensionSocket?.readyState === WebSocket.OPEN;
  }

  getConnectedTabs(): ConnectedTab[] {
    return Array.from(this.connectedTabs.values());
  }

  start(): Promise<void> {
    const port = parseInt(process.env.SP00KY_MCP_PORT || '', 10) || BRIDGE_PORT;

    return new Promise((resolve, reject) => {
      this.wss = new WebSocketServer({ host: '127.0.0.1', port }, () => {
        process.stderr.write(`[sp00ky-mcp] Bridge listening on ws://127.0.0.1:${port}\n`);
        resolve();
      });

      this.wss.on('error', (err) => {
        process.stderr.write(`[sp00ky-mcp] Bridge error: ${err.message}\n`);
        reject(err);
      });

      this.wss.on('connection', (ws) => {
        process.stderr.write('[sp00ky-mcp] Extension connected\n');

        // Only allow one extension connection at a time
        if (this.extensionSocket) {
          this.extensionSocket.close();
        }
        this.extensionSocket = ws;

        // Start keepalive pings
        this.startPing(ws);

        ws.on('message', (data) => {
          try {
            const msg = JSON.parse(data.toString());
            this.handleMessage(msg);
          } catch (err) {
            process.stderr.write(`[sp00ky-mcp] Bad message: ${err}\n`);
          }
        });

        ws.on('close', () => {
          process.stderr.write('[sp00ky-mcp] Extension disconnected\n');
          if (this.extensionSocket === ws) {
            this.extensionSocket = null;
            this.connectedTabs.clear();
            this.stopPing();
            // Reject all pending requests
            for (const [id, pending] of this.pendingRequests) {
              pending.reject(new Error('Extension disconnected'));
              clearTimeout(pending.timer);
              this.pendingRequests.delete(id);
            }
          }
        });

        ws.on('error', (err) => {
          process.stderr.write(`[sp00ky-mcp] Socket error: ${err.message}\n`);
        });
      });
    });
  }

  private startPing(ws: WebSocket) {
    this.stopPing();
    this.pingInterval = setInterval(() => {
      if (ws.readyState === WebSocket.OPEN) {
        ws.ping();
      }
    }, 20_000);
  }

  private stopPing() {
    if (this.pingInterval) {
      clearInterval(this.pingInterval);
      this.pingInterval = null;
    }
  }

  private handleMessage(msg: unknown) {
    // Handle response to a pending request
    if (isBridgeResponse(msg)) {
      const pending = this.pendingRequests.get(msg.id);
      if (pending) {
        clearTimeout(pending.timer);
        this.pendingRequests.delete(msg.id);
        if (msg.error) {
          pending.reject(new Error(msg.error.message));
        } else {
          pending.resolve(msg.result);
        }
      }
      return;
    }

    // Handle notifications from extension
    if (isBridgeNotification(msg)) {
      if (msg.method === 'tabsChanged') {
        this.connectedTabs.clear();
        const tabs = msg.params.tabs as ConnectedTab[];
        for (const tab of tabs) {
          this.connectedTabs.set(tab.tabId, tab);
        }
      }
      return;
    }
  }

  async request(method: string, params: Record<string, unknown> = {}, tabId?: number): Promise<unknown> {
    if (!this.extensionSocket || this.extensionSocket.readyState !== WebSocket.OPEN) {
      throw new Error('No extension connected. Make sure the Sp00ky DevTools extension is running and has a page with Sp00ky open.');
    }

    const id = `mcp-${++this.requestCounter}`;
    const resolvedTabId = tabId ?? this.getDefaultTabId();

    const request: BridgeRequest = {
      jsonrpc: '2.0',
      id,
      method,
      params,
      ...(resolvedTabId !== undefined ? { tabId: resolvedTabId } : {}),
    };

    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pendingRequests.delete(id);
        reject(new Error(`Request timed out after ${REQUEST_TIMEOUT_MS}ms: ${method}`));
      }, REQUEST_TIMEOUT_MS);

      this.pendingRequests.set(id, { resolve, reject, timer });
      this.extensionSocket!.send(JSON.stringify(request));
    });
  }

  private getDefaultTabId(): number | undefined {
    const tabs = this.getConnectedTabs();
    return tabs.length > 0 ? tabs[0].tabId : undefined;
  }

  async stop(): Promise<void> {
    this.stopPing();
    for (const [id, pending] of this.pendingRequests) {
      clearTimeout(pending.timer);
      pending.reject(new Error('Bridge shutting down'));
      this.pendingRequests.delete(id);
    }
    if (this.extensionSocket) {
      this.extensionSocket.close();
      this.extensionSocket = null;
    }
    return new Promise((resolve) => {
      if (this.wss) {
        this.wss.close(() => resolve());
      } else {
        resolve();
      }
    });
  }
}
