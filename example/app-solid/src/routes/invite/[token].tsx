import { createEffect, createSignal, Show } from 'solid-js';
import { useNavigate, useParams } from '@solidjs/router';
import { RecordId, useDb } from '@spooky-sync/client-solid';
import { useAuth } from '../../lib/auth';
import type { schema } from '../../schema.gen';

const parseRecordId = (id: string): RecordId => {
  const idx = id.indexOf(':');
  if (idx <= 0) throw new Error(`Invalid record id: ${id}`);
  return new RecordId(id.slice(0, idx), id.slice(idx + 1));
};

export default function InvitePage() {
  const params = useParams();
  const navigate = useNavigate();
  const db = useDb<typeof schema>();
  const auth = useAuth();
  const [error, setError] = createSignal<string | null>(null);
  const [accepted, setAccepted] = createSignal(false);

  createEffect(async () => {
    const userId = auth.userId();
    const token = params.token;
    if (!userId || !token || accepted()) return;

    try {
      const invite = await db.useRemote(async (s) => {
        const [rows] = await s.query<[{ thread: string | RecordId }[]]>(
          'SELECT thread FROM thread_invite WHERE token = $t LIMIT 1',
          { t: token }
        );
        return rows && rows.length > 0 ? rows[0] : null;
      });

      if (!invite?.thread) {
        setError('This invite link is invalid or has been revoked.');
        return;
      }

      const threadIdStr =
        invite.thread instanceof RecordId ? invite.thread.toString() : String(invite.thread);
      const threadRid = parseRecordId(threadIdStr);
      const userRid = parseRecordId(userId);

      await db.useRemote(async (s) => {
        try {
          await s.query('RELATE $u->collaborates_on->$th', { u: userRid, th: threadRid });
        } catch (e: any) {
          const msg = (e?.message || '').toLowerCase();
          const isDup =
            msg.includes('unique') ||
            msg.includes('already exists') ||
            msg.includes('duplicate');
          if (!isDup) throw e;
        }
      });

      setAccepted(true);
      const suffix = threadIdStr.split(':').slice(1).join(':');
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
            <p class="text-sm text-zinc-500">Hang tight.</p>
          </Show>
        </Show>
      </div>
    </div>
  );
}
