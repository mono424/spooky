import { createResource, createSignal, For, Show } from 'solid-js';
import { RecordId, useDb, useQuery } from '@spooky-sync/client-solid';
import * as jose from 'jose';
import QRCode from 'qrcode';
import type { schema } from '../schema.gen';
import { useAuth } from '../lib/auth';
import { createHotkey } from '../lib/keyboard';
import { Tooltip } from './Tooltip';

interface ShareDialogProps {
  threadId: string;
  isOpen: boolean;
  onClose: () => void;
}

const SHARE_TTL_DAYS = 7;

const parseRecordId = (id: string | RecordId): RecordId => {
  // `auth.userId()` is typed `string` but actually emits the SurrealDB SDK's
  // `RecordId` object at runtime. Pass that through; only parse the colon
  // form when we genuinely got a string.
  if (id instanceof RecordId) return id;
  const s = String(id);
  const idx = s.indexOf(':');
  if (idx <= 0) throw new Error(`Invalid record id: ${s}`);
  return new RecordId(s.slice(0, idx), s.slice(idx + 1));
};

const inviteUrl = (jwt: string) => `${window.location.origin}/invite/${jwt}`;

function genJti(): string {
  // 16 random bytes → base64url, plenty for a uniqueness nonce.
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  let s = '';
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

export function ShareDialog(props: ShareDialogProps) {
  const db = useDb<typeof schema>();
  const auth = useAuth();
  const [isCreating, setIsCreating] = createSignal(false);
  const [copiedId, setCopiedId] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  // Local-first query — pulls from the synced cache, so the dialog renders
  // even when offline. The schema's `share_link` SELECT rule already scopes
  // to the issuer, no extra filter needed beyond the thread.
  const linksQuery = useQuery(
    () => {
      if (!props.isOpen) return null;
      return db
        .query('share_link')
        .where({ thread: props.threadId })
        .build();
    },
    { enabled: () => props.isOpen },
  );

  createHotkey('Escape', () => props.onClose(), () => ({ enabled: props.isOpen, ignoreInputs: false }));

  const create = async () => {
    if (isCreating()) return;
    setIsCreating(true);
    setError(null);
    try {
      const userId = auth.userId();
      if (!userId) throw new Error('Not authenticated.');

      // Read the private key from the `user_keypair` table. Its table-level
      // SELECT rule (`owner = $auth`) is what protects the key from other
      // users; we still rely on the synced local cache so this works
      // offline.
      const [keypairRow] = await db.useRemote(async (s) =>
        s.query<[Array<{ privkey: string }>]>(
          'SELECT privkey FROM user_keypair WHERE owner = $u LIMIT 1',
          { u: parseRecordId(userId) },
        ),
      );
      const privPem = keypairRow?.[0]?.privkey;
      if (!privPem) {
        throw new Error("Couldn't find your sharing key. Try reloading after a brief reconnect.");
      }

      // Sign the JWT locally — no network needed.
      const privKey = await jose.importPKCS8(privPem, 'EdDSA');
      const expSeconds = Math.floor(Date.now() / 1000) + SHARE_TTL_DAYS * 24 * 3600;
      const jti = genJti();
      const jwt = await new jose.SignJWT({ jti })
        .setProtectedHeader({ alg: 'EdDSA' })
        .setIssuer(String(userId))
        .setSubject(String(props.threadId))
        .setIssuedAt()
        .setExpirationTime(expSeconds)
        .sign(privKey);

      // Persist in `share_link` so the UI lists it. The remote permission
      // rule (only the thread author may CREATE) double-gates the write.
      await db.useRemote(async (s) =>
        s.query(
          `CREATE share_link SET thread = $thread, jwt = $jwt, jti = $jti, exp = $exp`,
          {
            thread: parseRecordId(props.threadId),
            jwt,
            jti,
            exp: new Date(expSeconds * 1000),
          },
        ),
      );
    } catch (e: any) {
      setError(e?.message || 'Failed to create share link.');
    } finally {
      setIsCreating(false);
    }
  };

  const copy = async (link: { id: string; jwt: string }) => {
    try {
      await navigator.clipboard.writeText(inviteUrl(link.jwt));
      setCopiedId(link.id);
      setTimeout(() => setCopiedId((cur) => (cur === link.id ? null : cur)), 1500);
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

            <div class="rounded-lg border border-amber-500/20 bg-amber-500/5 px-3 py-2 text-xs text-amber-300/90">
              Share links can't be revoked. They expire {SHARE_TTL_DAYS} days after they're created.
            </div>

            <button
              onMouseDown={create}
              disabled={isCreating()}
              class="w-full bg-surface hover:bg-surface-hover border border-white/[0.06] text-zinc-300 hover:text-white py-2.5 px-4 rounded-lg font-medium transition-colors duration-150 disabled:opacity-50 disabled:cursor-not-allowed text-sm"
            >
              {isCreating() ? 'Creating...' : 'Create share link'}
            </button>

            <Show when={error()}>
              <div class="bg-red-500/10 border border-red-500/20 rounded-lg text-red-400 p-3 text-sm">
                {error()}
              </div>
            </Show>

            <div class="space-y-3">
              <For
                each={linksQuery.data() ?? []}
                fallback={
                  <div class="text-center text-sm text-zinc-600 py-6">No share links yet.</div>
                }
              >
                {(link) => <ShareLinkRow link={link} copiedId={copiedId()} onCopy={copy} />}
              </For>
            </div>
          </div>
        </div>
      </div>
    </Show>
  );
}

interface ShareLinkRow {
  id: any;
  jwt: string;
  exp?: any;
}

function ShareLinkRow(props: {
  link: ShareLinkRow;
  copiedId: string | null;
  onCopy: (link: { id: string; jwt: string }) => void;
}) {
  const idStr = () =>
    typeof props.link.id === 'string'
      ? props.link.id
      : `${props.link.id.tb}:${String(props.link.id.id)}`;

  const [qrSvg] = createResource(
    () => props.link.jwt,
    async (jwt) => {
      try {
        return await QRCode.toString(`${window.location.origin}/invite/${jwt}`, {
          type: 'svg',
          margin: 1,
          width: 160,
          color: { dark: '#e4e4e7', light: '#0000' },
        });
      } catch {
        return '';
      }
    },
  );

  return (
    <div class="bg-zinc-950 border border-white/[0.06] rounded-lg px-3 py-3 space-y-2">
      <div class="flex items-center gap-2">
        <input
          readOnly
          value={`${window.location.origin}/invite/${props.link.jwt}`}
          class="flex-1 bg-transparent outline-none text-xs text-zinc-300 font-mono truncate"
          onFocus={(e) => e.currentTarget.select()}
        />
        <button
          onMouseDown={() => props.onCopy({ id: idStr(), jwt: props.link.jwt })}
          class="text-xs font-medium bg-surface hover:bg-surface-hover border border-white/[0.06] text-zinc-300 hover:text-white px-3 py-1 rounded-md transition-colors duration-150"
        >
          {props.copiedId === idStr() ? 'Copied' : 'Copy'}
        </button>
      </div>
      <Show when={qrSvg()}>
        <div class="flex justify-center pt-1" innerHTML={qrSvg()} />
      </Show>
    </div>
  );
}
