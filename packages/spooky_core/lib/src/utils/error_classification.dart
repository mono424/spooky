const _networkErrorPatterns = [
  'connection',
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
