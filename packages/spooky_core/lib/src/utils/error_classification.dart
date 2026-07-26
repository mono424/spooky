const _networkErrorPatterns = [
  'connection',
  // Surreal's ConnectionUnavailableError reads "You must be connected to a
  // SurrealDB instance..." — it contains "connected", not "connection", so it
  // slips past the pattern above. The WS client throws it while the socket is
  // down but reconnect hasn't fired yet, so it's the canonical error on an
  // idle-dropped socket: classify it as network so the mutation is re-queued
  // rather than rolled back and dropped.
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

/// Classify a sync error as recoverable (`network`) or terminal
/// (`application`). Faithful port of TS `classifySyncError`.
String classifySyncError(Object? error) {
  final message =
      (error is Error ? error.toString() : error.toString()).toLowerCase();
  for (final pattern in _networkErrorPatterns) {
    if (message.contains(pattern)) {
      return 'network';
    }
  }
  return 'application';
}
