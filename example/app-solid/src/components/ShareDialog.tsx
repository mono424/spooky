import { createEffect, createSignal, For, Show } from 'solid-js';
import { RecordId, useDb, Uuid } from '@spooky-sync/client-solid';
import type { schema } from '../schema.gen';
import { createHotkey } from '../lib/keyboard';
import { Tooltip } from './Tooltip';

interface ShareDialogProps {
  threadId: string;
  isOpen: boolean;
  onClose: () => void;
}

interface InviteRow {
  id: string;
  token: string;
  created_at: string;
}

const parseRecordId = (id: string): RecordId => {
  const idx = id.indexOf(':');
  if (idx <= 0) throw new Error(`Invalid record id: ${id}`);
  return new RecordId(id.slice(0, idx), id.slice(idx + 1));
};

const inviteUrl = (token: string) => `${window.location.origin}/invite/${token}`;

export function ShareDialog(props: ShareDialogProps) {
  const db = useDb<typeof schema>();
  const [invites, setInvites] = createSignal<InviteRow[]>([]);
  const [isCreating, setIsCreating] = createSignal(false);
  const [copiedId, setCopiedId] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  const refresh = async () => {
    setError(null);
    try {
      const result = await db.useRemote(async (s) =>
        s.query<[InviteRow[]]>(
          'SELECT id, token, created_at FROM thread_invite WHERE thread = $t ORDER BY created_at DESC',
          { t: parseRecordId(props.threadId) }
        )
      );
      const rows = result?.[0] ?? [];
      setInvites(
        rows.map((r: any) => ({
          id: typeof r.id === 'string' ? r.id : `${r.id.tb}:${String(r.id.id)}`,
          token: r.token,
          created_at: r.created_at,
        }))
      );
    } catch (e: any) {
      setError(e?.message || 'Failed to load invites.');
    }
  };

  createEffect(() => {
    if (props.isOpen) refresh();
  });

  createHotkey('Escape', () => props.onClose(), () => ({ enabled: props.isOpen, ignoreInputs: false }));

  const create = async () => {
    if (isCreating()) return;
    setIsCreating(true);
    setError(null);
    try {
      const token = Uuid.v4().toString().replace(/-/g, '');
      await db.useRemote(async (s) =>
        s.query('CREATE thread_invite SET thread = $thread, token = $token', {
          thread: parseRecordId(props.threadId),
          token,
        })
      );
      await refresh();
    } catch (e: any) {
      setError(e?.message || 'Failed to create invite.');
    } finally {
      setIsCreating(false);
    }
  };

  const revoke = async (inviteId: string) => {
    setError(null);
    try {
      await db.useRemote(async (s) => s.delete(parseRecordId(inviteId)));
      await refresh();
    } catch (e: any) {
      setError(e?.message || 'Failed to revoke invite.');
    }
  };

  const copy = async (invite: InviteRow) => {
    try {
      await navigator.clipboard.writeText(inviteUrl(invite.token));
      setCopiedId(invite.id);
      setTimeout(() => setCopiedId((cur) => (cur === invite.id ? null : cur)), 1500);
    } catch (e: any) {
      setError(e?.message || 'Failed to copy.');
    }
  };

  return (
    <Show when={props.isOpen}>
      <div
        class="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-[100] p-4"
        onMouseDown={props.onClose}
      >
        <div
          class="animate-slide-up bg-surface border border-white/[0.06] rounded-xl w-full max-w-lg shadow-2xl"
          onMouseDown={(e) => e.stopPropagation()}
        >
          <div class="flex justify-between items-center px-6 pt-6 pb-2">
            <h2 class="text-lg font-semibold">Share thread</h2>
            <Tooltip text="Close" kbd="Esc">
              <button
                onMouseDown={props.onClose}
                class="text-zinc-500 hover:text-white transition-colors duration-150 p-1"
                aria-label="Close"
              >
                <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    stroke-width="2"
                    d="M6 18L18 6M6 6l12 12"
                  />
                </svg>
              </button>
            </Tooltip>
          </div>

          <div class="px-6 pb-6 pt-2 space-y-4">
            <p class="text-sm text-zinc-500">
              Anyone signed in who opens an invite link is added as an editor.
            </p>

            <button
              onMouseDown={create}
              disabled={isCreating()}
              class="w-full bg-surface hover:bg-surface-hover border border-white/[0.06] text-zinc-300 hover:text-white py-2.5 px-4 rounded-lg font-medium transition-colors duration-150 disabled:opacity-50 disabled:cursor-not-allowed text-sm"
            >
              {isCreating() ? 'Creating...' : 'Create invite link'}
            </button>

            <Show when={error()}>
              <div class="bg-red-500/10 border border-red-500/20 rounded-lg text-red-400 p-3 text-sm">
                {error()}
              </div>
            </Show>

            <div class="space-y-2">
              <For
                each={invites()}
                fallback={
                  <div class="text-center text-sm text-zinc-600 py-6">
                    No invite links yet.
                  </div>
                }
              >
                {(invite) => (
                  <div class="flex items-center gap-2 bg-zinc-950 border border-white/[0.06] rounded-lg px-3 py-2">
                    <input
                      readOnly
                      value={inviteUrl(invite.token)}
                      class="flex-1 bg-transparent outline-none text-xs text-zinc-300 font-mono truncate"
                      onFocus={(e) => e.currentTarget.select()}
                    />
                    <button
                      onMouseDown={() => copy(invite)}
                      class="text-xs font-medium bg-surface hover:bg-surface-hover border border-white/[0.06] text-zinc-300 hover:text-white px-3 py-1 rounded-md transition-colors duration-150"
                    >
                      {copiedId() === invite.id ? 'Copied' : 'Copy'}
                    </button>
                    <button
                      onMouseDown={() => revoke(invite.id)}
                      class="text-xs font-medium text-zinc-500 hover:text-red-400 px-2 py-1 transition-colors duration-150"
                      title="Revoke"
                    >
                      Revoke
                    </button>
                  </div>
                )}
              </For>
            </div>
          </div>
        </div>
      </div>
    </Show>
  );
}
