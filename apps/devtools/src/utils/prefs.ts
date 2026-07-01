/**
 * Tiny localStorage-backed preferences for the DevTools panel. Values are
 * JSON-encoded and namespaced so they survive closing/reopening DevTools.
 * All access is guarded — a disabled/full localStorage just falls back.
 */
const PREFIX = 'sp00ky-devtools:';

export function getPref<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(PREFIX + key);
    if (raw === null) return fallback;
    return JSON.parse(raw) as T;
  } catch {
    return fallback;
  }
}

export function setPref<T>(key: string, value: T): void {
  try {
    localStorage.setItem(PREFIX + key, JSON.stringify(value));
  } catch {
    // ignore — preferences are best-effort
  }
}
