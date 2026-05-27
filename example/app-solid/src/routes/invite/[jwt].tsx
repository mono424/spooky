import { createEffect, createSignal, Show } from 'solid-js';
import { useNavigate, useParams } from '@solidjs/router';
import { useDb } from '@spooky-sync/client-solid';
import * as jose from 'jose';
import { useAuth } from '../../lib/auth';
import type { schema } from '../../schema.gen';

const API_URL = import.meta.env.VITE_API_URL || 'http://localhost:3660';

export default function InvitePage() {
  const params = useParams();
  const navigate = useNavigate();
  const db = useDb<typeof schema>();
  const auth = useAuth();
  const [error, setError] = createSignal<string | null>(null);
  const [accepted, setAccepted] = createSignal(false);

  createEffect(async () => {
    const userId = auth.userId();
    const jwt = params.jwt;
    if (!userId || !jwt || accepted()) return;

    // Pull the recipient's Surreal session token off the live connection so
    // the API can identify them. The API uses a separate `Surreal` client
    // signed in with that token to derive `$auth.id`.
    let recipientToken: string | undefined;
    try {
      recipientToken = await db.useRemote(async (s) => s.accessToken);
    } catch (e: any) {
      setError(e?.message || 'Could not read session token.');
      return;
    }

    if (!recipientToken) {
      setError('Missing session token. Try signing in again.');
      return;
    }

    try {
      const resp = await fetch(`${API_URL}/share/accept`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Authorization: `Bearer ${recipientToken}`,
        },
        body: JSON.stringify({ jwt }),
      });
      if (!resp.ok) {
        const body = await resp.json().catch(() => ({}));
        const msg = body?.error || `Server returned ${resp.status}.`;
        if (resp.status === 401 || resp.status === 403) {
          setError(`This invite link is invalid or has expired. (${msg})`);
        } else {
          setError(msg);
        }
        return;
      }
      const { thread } = (await resp.json()) as { thread: string };

      // Best-effort sanity decode so we can show a useful error when the
      // server says one thread but the JWT carries another (shouldn't happen
      // — they're the same — but cheap to verify).
      try {
        const claims = jose.decodeJwt(jwt);
        if (typeof claims.sub === 'string' && claims.sub !== thread) {
          console.warn('[invite] thread mismatch', { sub: claims.sub, thread });
        }
      } catch {
        /* ignore decode errors — server already accepted it */
      }

      setAccepted(true);
      const suffix = thread.split(':').slice(1).join(':');
      navigate(`/thread/${suffix}`, { replace: true });
    } catch (e: any) {
      console.error('[invite] failed to accept:', e);
      setError(e?.message || 'Failed to accept invite.');
    }
  });

  return (
    <div class="min-h-[60vh] flex items-center justify-center px-6">
      <div class="bg-surface/50 rounded-xl border border-white/[0.06] p-10 text-center max-w-sm">
        <Show
          when={auth.userId()}
          fallback={
            <>
              <p class="text-zinc-200 font-medium mb-2">You've been invited to a thread.</p>
              <p class="text-sm text-zinc-500">Sign in to accept the invite.</p>
            </>
          }
        >
          <Show
            when={!error()}
            fallback={
              <>
                <p class="text-zinc-200 font-medium mb-2">Invite error</p>
                <p class="text-sm text-zinc-500">{error()}</p>
              </>
            }
          >
            <p class="text-zinc-200 font-medium mb-2">Joining thread...</p>
            <p class="text-sm text-zinc-500">Verifying invite.</p>
          </Show>
        </Show>
      </div>
    </div>
  );
}
