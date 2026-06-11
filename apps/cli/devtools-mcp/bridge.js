import { WebSocketServer, WebSocket } from 'ws';
import { isBridgeResponse, isBridgeNotification, BRIDGE_PORT, } from './protocol.js';
const REQUEST_TIMEOUT_MS = 10_000;
export class Bridge {
    wss = null;
    extensionSocket = null;
    connectedTabs = new Map();
    pendingRequests = new Map();
    requestCounter = 0;
    pingInterval = null;
    get isConnected() {
        return this.extensionSocket?.readyState === WebSocket.OPEN;
    }
    getConnectedTabs() {
        return Array.from(this.connectedTabs.values());
    }
    start() {
        const port = Number.parseInt(process.env.SP00KY_MCP_PORT || '', 10) || BRIDGE_PORT;
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
                    }
                    catch (err) {
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
    startPing(ws) {
        this.stopPing();
        this.pingInterval = setInterval(() => {
            if (ws.readyState === WebSocket.OPEN) {
                ws.ping();
            }
        }, 20_000);
    }
    stopPing() {
        if (this.pingInterval) {
            clearInterval(this.pingInterval);
            this.pingInterval = null;
        }
    }
    handleMessage(msg) {
        // Handle response to a pending request
        if (isBridgeResponse(msg)) {
            const pending = this.pendingRequests.get(msg.id);
            if (pending) {
                clearTimeout(pending.timer);
                this.pendingRequests.delete(msg.id);
                if (msg.error) {
                    pending.reject(new Error(msg.error.message));
                }
                else {
                    pending.resolve(msg.result);
                }
            }
            return;
        }
        // Handle notifications from extension
        if (isBridgeNotification(msg)) {
            if (msg.method === 'tabsChanged') {
                this.connectedTabs.clear();
                const tabs = msg.params.tabs;
                for (const tab of tabs) {
                    this.connectedTabs.set(tab.tabId, tab);
                }
            }
            return;
        }
    }
    async request(method, params = {}, tabId) {
        if (!this.extensionSocket || this.extensionSocket.readyState !== WebSocket.OPEN) {
            throw new Error('No extension connected. Make sure the Sp00ky DevTools extension is running and has a page with Sp00ky open.');
        }
        const id = `mcp-${++this.requestCounter}`;
        const resolvedTabId = tabId ?? this.getDefaultTabId();
        const request = {
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
            // oxlint-disable-next-line no-non-null-assertion
            this.extensionSocket.send(JSON.stringify(request));
        });
    }
    getDefaultTabId() {
        const tabs = this.getConnectedTabs();
        return tabs.length > 0 ? tabs[0].tabId : undefined;
    }
    async stop() {
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
            }
            else {
                resolve();
            }
        });
    }
}
