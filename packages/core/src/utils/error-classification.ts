const NETWORK_ERROR_PATTERNS = [
  'connection',
  // surreal's ConnectionUnavailableError reads "You must be connected to a
  // SurrealDB instance..." — it contains "connected", not "connection", so it
  // slips past the pattern above. The WS engine throws it synchronously from
  // `send()` while the socket is down but reconnect hasn't fired yet, so it's the
  // canonical error on an idle-dropped socket; classify it as network.
  'must be connected',
  'connectionunavailable',
  'timeout',
  'timed out',
  'websocket',
  'fetch failed',
  'disconnected',
  'socket',
  'network',
  'econnrefused',
  'econnreset',
  'enotfound',
  'epipe',
  'abort',
];

export function classifySyncError(error: unknown): 'network' | 'application' {
  const message =
    error instanceof Error ? error.message.toLowerCase() : String(error).toLowerCase();

  for (const pattern of NETWORK_ERROR_PATTERNS) {
    if (message.includes(pattern)) {
      return 'network';
    }
  }

  return 'application';
}
