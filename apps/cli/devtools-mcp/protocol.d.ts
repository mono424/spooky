export interface BridgeRequest {
    jsonrpc: '2.0';
    id: string;
    method: string;
    params: Record<string, unknown>;
    tabId?: number;
}
export interface BridgeResponse {
    jsonrpc: '2.0';
    id: string;
    result?: unknown;
    error?: {
        code: number;
        message: string;
    };
}
export interface BridgeNotification {
    jsonrpc: '2.0';
    method: string;
    params: Record<string, unknown>;
}
export type BridgeMessage = BridgeRequest | BridgeResponse | BridgeNotification;
export declare const BRIDGE_METHODS: {
    readonly GET_STATE: "getState";
    readonly RUN_QUERY: "runQuery";
    readonly GET_TABLE_DATA: "getTableData";
    readonly UPDATE_TABLE_ROW: "updateTableRow";
    readonly DELETE_TABLE_ROW: "deleteTableRow";
    readonly CLEAR_HISTORY: "clearHistory";
};
export declare const BRIDGE_PORT = 9315;
export declare function isBridgeResponse(msg: unknown): msg is BridgeResponse;
export declare function isBridgeRequest(msg: unknown): msg is BridgeRequest;
export declare function isBridgeNotification(msg: unknown): msg is BridgeNotification;
