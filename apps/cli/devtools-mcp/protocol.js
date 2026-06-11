// Shared message types for MCP Server <-> Chrome Extension bridge (JSON-RPC 2.0 style)
// Methods the MCP server can call on the extension
export const BRIDGE_METHODS = {
    GET_STATE: 'getState',
    RUN_QUERY: 'runQuery',
    GET_TABLE_DATA: 'getTableData',
    UPDATE_TABLE_ROW: 'updateTableRow',
    DELETE_TABLE_ROW: 'deleteTableRow',
    CLEAR_HISTORY: 'clearHistory',
};
export const BRIDGE_PORT = 9315;
export function isBridgeResponse(msg) {
    return (typeof msg === 'object' &&
        msg !== null &&
        'jsonrpc' in msg &&
        msg.jsonrpc === '2.0' &&
        'id' in msg &&
        ('result' in msg || 'error' in msg));
}
export function isBridgeRequest(msg) {
    return (typeof msg === 'object' &&
        msg !== null &&
        'jsonrpc' in msg &&
        msg.jsonrpc === '2.0' &&
        'method' in msg &&
        'id' in msg &&
        !('result' in msg) &&
        !('error' in msg));
}
export function isBridgeNotification(msg) {
    return (typeof msg === 'object' &&
        msg !== null &&
        'jsonrpc' in msg &&
        msg.jsonrpc === '2.0' &&
        'method' in msg &&
        !('id' in msg));
}
