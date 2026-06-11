interface ConnectedTab {
    tabId: number;
    url?: string;
    title?: string;
}
export declare class Bridge {
    private wss;
    private extensionSocket;
    private connectedTabs;
    private pendingRequests;
    private requestCounter;
    private pingInterval;
    get isConnected(): boolean;
    getConnectedTabs(): ConnectedTab[];
    start(): Promise<void>;
    private startPing;
    private stopPing;
    private handleMessage;
    request(method: string, params?: Record<string, unknown>, tabId?: number): Promise<unknown>;
    private getDefaultTabId;
    stop(): Promise<void>;
}
export {};
