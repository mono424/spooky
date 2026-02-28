const NETWORK_ERROR_PATTERNS = [
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
