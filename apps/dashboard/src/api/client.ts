/**
 * The single place that knows where the scheduler is and how to prove who we
 * are.
 *
 * # Embedded vs standalone
 *
 * The same bundle is served by a scheduler at `/admin` and can also be opened
 * from anywhere and pointed at a scheduler by URL. It works out which it is by
 * asking: a successful same-origin `GET /admin/api/config` means a scheduler is
 * serving us, so the base URL is '' and the login form hides its endpoint
 * field. Anything else means standalone.
 *
 * We detect rather than have the server inject a global, because detection has
 * one code path and one failure mode. An injected `window.__SPKY_ENDPOINT__`
 * would need the scheduler to rewrite index.html on the way out, which turns a
 * static file server into a templating engine for no gain.
 *
 * # Why a bearer token and not a cookie
 *
 * Standalone means cross-origin, and a cross-origin cookie needs
 * `SameSite=None; Secure`. Browsers refuse that over plain http, which is
 * exactly how a scheduler at an IP address is reached. A bearer token in
 * localStorage behaves identically in both modes and takes CSRF off the table.
 */

import type { LoginResponse, ServerConfig } from './types';

const ENDPOINT_KEY = 'spky.endpoint';
const TOKEN_PREFIX = 'spky.token:';

/** localStorage is unavailable in some privacy modes; never let that be fatal. */
function readStore(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeStore(key: string, value: string | null) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    /* ignore */
  }
}

/** Thrown for any non-2xx response, carrying the status so callers can branch. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

export interface Mode {
  embedded: boolean;
  /** '' when embedded (same origin), otherwise an absolute scheduler URL. */
  baseUrl: string;
  config: ServerConfig | null;
}

let mode: Mode = { embedded: false, baseUrl: '', config: null };

export function currentMode(): Mode {
  return mode;
}

export function normalizeEndpoint(raw: string): string {
  const trimmed = raw.trim().replace(/\/+$/, '');
  if (!trimmed) return '';
  // Bare host:port is the shape an operator actually types.
  return /^https?:\/\//i.test(trimmed) ? trimmed : `http://${trimmed}`;
}

async function fetchConfig(baseUrl: string): Promise<ServerConfig> {
  const res = await fetch(`${baseUrl}/admin/api/config`);
  if (!res.ok) throw new ApiError(res.status, `config responded ${res.status}`);
  return (await res.json()) as ServerConfig;
}

/**
 * Work out embedded vs standalone. Called once at boot, and again whenever the
 * operator changes the endpoint in the login form.
 */
export async function resolveMode(): Promise<Mode> {
  try {
    const config = await fetchConfig('');
    mode = { embedded: true, baseUrl: '', config };
    return mode;
  } catch {
    // Not served by a scheduler. Fall back to a saved endpoint, if any; its
    // config is fetched lazily so a saved-but-unreachable endpoint still lets
    // the login form render and be corrected.
    const saved = readStore(ENDPOINT_KEY) ?? '';
    mode = { embedded: false, baseUrl: saved, config: null };
    if (saved) {
      try {
        mode = { ...mode, config: await fetchConfig(saved) };
      } catch {
        /* leave config null; the form will show the endpoint field */
      }
    }
    return mode;
  }
}

export async function setEndpoint(raw: string): Promise<ServerConfig> {
  const baseUrl = normalizeEndpoint(raw);
  const config = await fetchConfig(baseUrl);
  writeStore(ENDPOINT_KEY, baseUrl);
  mode = { embedded: false, baseUrl, config };
  return config;
}

/** Tokens are keyed by endpoint, so two schedulers in two tabs don't collide. */
function tokenKey(): string {
  return `${TOKEN_PREFIX}${mode.baseUrl}`;
}

export function getToken(): string | null {
  return readStore(tokenKey());
}

export function setToken(token: string | null) {
  writeStore(tokenKey(), token);
}

/** Listeners fired when the server rejects our token, so the app can sign out. */
const unauthorizedHandlers = new Set<() => void>();

export function onUnauthorized(fn: () => void): () => void {
  unauthorizedHandlers.add(fn);
  return () => unauthorizedHandlers.delete(fn);
}

function notifyUnauthorized() {
  setToken(null);
  unauthorizedHandlers.forEach((fn) => fn());
}

export function url(path: string): string {
  return `${mode.baseUrl}/admin/api${path}`;
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const token = getToken();
  const headers = new Headers(init.headers);
  if (token) headers.set('Authorization', `Bearer ${token}`);
  if (init.body) headers.set('Content-Type', 'application/json');

  const res = await fetch(url(path), { ...init, headers });

  if (res.status === 401) {
    notifyUnauthorized();
    throw new ApiError(401, 'Session expired');
  }
  if (!res.ok) {
    // The API answers errors as {error} JSON; fall back to the raw body for
    // anything that does not (a proxy's own error page, say).
    let message = `Request failed (${res.status})`;
    try {
      const body = await res.json();
      if (body && typeof body.error === 'string') message = body.error;
    } catch {
      /* keep the default */
    }
    throw new ApiError(res.status, message);
  }
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

/** A request that reports failure as a value instead of throwing. */
export type Result<T> =
  | { ok: true; value: T }
  | { ok: false; message: string };

export const api = {
  get: <T>(path: string) => request<T>(path),

  /**
   * `get`, but failures come back as data.
   *
   * Solid re-throws a rejected `createResource` fetcher during render, and
   * without an `ErrorBoundary` above it that leaves the subtree wedged — a 404
   * renders as a permanent "Loading…" rather than a message. Views that fetch
   * a single addressable thing use this so the failure is an ordinary branch.
   *
   * A 401 still runs the unauthorized handlers first, inside `request`, so
   * session expiry keeps ejecting to the login screen.
   */
  getResult: async <T>(path: string): Promise<Result<T>> => {
    try {
      return { ok: true, value: await request<T>(path) };
    } catch (err) {
      return {
        ok: false,
        message: err instanceof Error ? err.message : 'Request failed',
      };
    }
  },
  post: <T>(path: string, body?: unknown) =>
    request<T>(path, {
      method: 'POST',
      body: body === undefined ? undefined : JSON.stringify(body),
    }),
};

export async function login(
  password: string,
  username?: string,
): Promise<LoginResponse> {
  // Not through `request`: a failed login must not fire the unauthorized
  // handlers, which exist to eject an already-signed-in session.
  const res = await fetch(url('/session'), {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ username, password }),
  });

  if (!res.ok) {
    let message = `Sign-in failed (${res.status})`;
    try {
      const body = await res.json();
      if (body && typeof body.error === 'string') message = body.error;
    } catch {
      /* keep the default */
    }
    throw new ApiError(res.status, message);
  }

  const out = (await res.json()) as LoginResponse;
  setToken(out.token);
  return out;
}

export async function logout() {
  try {
    await api.post('/logout');
  } catch {
    // A server that will not take the logout is not a reason to keep a token
    // the operator has asked us to drop.
  }
  setToken(null);
}

/**
 * Open an SSE stream with our bearer attached.
 *
 * `EventSource` cannot send an Authorization header — its only auth mechanism
 * is cookies, which is the thing we deliberately do not use — so this reads the
 * `fetch` body and parses the event stream by hand. It is a small format and
 * this is the whole of it: `event:` names the type, `data:` carries the
 * payload, a blank line ends a frame, and a leading `:` is a keep-alive
 * comment.
 */
export function openStream(
  path: string,
  handlers: {
    onEvent: (event: string, data: string) => void;
    onError?: (err: unknown) => void;
    onOpen?: () => void;
  },
): () => void {
  const controller = new AbortController();
  let closed = false;

  (async () => {
    const token = getToken();
    const headers = new Headers();
    if (token) headers.set('Authorization', `Bearer ${token}`);

    try {
      const res = await fetch(url(path), {
        headers,
        signal: controller.signal,
      });

      if (res.status === 401) {
        notifyUnauthorized();
        return;
      }
      if (!res.ok || !res.body) {
        throw new ApiError(res.status, `Stream failed (${res.status})`);
      }
      handlers.onOpen?.();

      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';

      while (!closed) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });

        // Frames are separated by a blank line. Anything after the last one is
        // a partial frame and stays in the buffer for the next read.
        let split: number;
        while ((split = buffer.indexOf('\n\n')) !== -1) {
          const frame = buffer.slice(0, split);
          buffer = buffer.slice(split + 2);

          let eventName = 'message';
          const dataLines: string[] = [];
          for (const line of frame.split('\n')) {
            if (line.startsWith(':')) continue; // keep-alive
            if (line.startsWith('event:')) eventName = line.slice(6).trim();
            else if (line.startsWith('data:')) dataLines.push(line.slice(5).trimStart());
          }
          if (dataLines.length) handlers.onEvent(eventName, dataLines.join('\n'));
        }
      }
    } catch (err) {
      // An abort is us closing the stream, not a failure.
      if (!closed && (err as { name?: string })?.name !== 'AbortError') {
        handlers.onError?.(err);
      }
    }
  })();

  return () => {
    closed = true;
    controller.abort();
  };
}
