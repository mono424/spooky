import { LoroDoc } from 'loro-crdt';
import type { LocalDatabaseService, RemoteDatabaseService } from '../../services/database/index';
import type { Logger } from '../../services/logger/index';
import { parseRecordIdString } from '../../utils/index';

// ==================== CURSOR UTILITIES ====================

export const CURSOR_COLORS = [
  '#3b82f6', '#ef4444', '#22c55e', '#f59e0b',
  '#8b5cf6', '#ec4899', '#14b8a6', '#f97316',
];

export function cursorColorFromName(name: string): string {
  let hash = 0;
  for (let i = 0; i < name.length; i++) {
    hash = ((hash << 5) - hash + name.charCodeAt(i)) | 0;
  }
  return CURSOR_COLORS[Math.abs(hash) % CURSOR_COLORS.length];
}

// ==================== CRDT FIELD ====================

export class CrdtField {
  private doc: LoroDoc;
  private pushTimer: ReturnType<typeof setTimeout> | null = null;
  private local: LocalDatabaseService | null = null;
  private remote: RemoteDatabaseService | null = null;
  private recordId: string | null = null;
  private sessionId: string = '';
  private unsubscribe: (() => void) | null = null;
  private lastPushTime = 0;
  private lastCursorPushTime = 0;
  private loadedFromCrdt = false;
  private pushRetryCount = 0;
  private logger: Logger | null;
  private cursorsEnabled: boolean;
  /** Remote-push debounce. Local writes happen immediately on every Loro
   *  update; the remote UPSERT is coalesced over this window. Configured
   *  via `Sp00kyConfig.crdtDebounceMs`, default 500. */
  private remoteDebounceMs: number = 500;

  private _onCursorUpdate: ((data: Uint8Array) => void) | null = null;
  private pendingCursorUpdate: Uint8Array | null = null;

  /** Callback set by the editor to receive remote cursor updates.
   *  Any cursor data that arrived before this callback was set will be replayed. */
  set onCursorUpdate(cb: ((data: Uint8Array) => void) | null) {
    this._onCursorUpdate = cb;
    if (cb && this.pendingCursorUpdate) {
      try { cb(this.pendingCursorUpdate); } catch (e) {
        this.logger?.warn(
          { error: e, Category: 'sp00ky-client::CrdtField::onCursorUpdate' },
          'Failed to replay pending cursor update'
        );
      }
      this.pendingCursorUpdate = null;
    }
  }

  get onCursorUpdate() { return this._onCursorUpdate; }

  constructor(
    private fieldName: string,
    cursorsEnabled: boolean,
    initialState?: Uint8Array,
    logger?: Logger | null,
  ) {
    if (!/^[a-zA-Z_][a-zA-Z0-9_]*$/.test(fieldName)) {
      throw new Error(
        `CrdtField: refusing unsafe field identifier '${fieldName}' — must match [a-zA-Z_][a-zA-Z0-9_]*`
      );
    }
    this.logger = logger ?? null;
    this.cursorsEnabled = cursorsEnabled;
    this.doc = new LoroDoc();
    if (initialState && initialState.length > 0) {
      // Tolerance: catch bad-snapshot data (corrupt blob, stale legacy
      // value left over from a pre-`bytes` migration) so the editor still
      // mounts. Without this guard the rejection bubbles through
      // `useCrdtField` → permanent fallback `<p>`, with no cursor.
      try {
        this.doc.import(initialState);
        this.loadedFromCrdt = true;
      } catch (e) {
        this.logger?.warn(
          {
            error: e,
            fieldName,
            Category: 'sp00ky-client::CrdtField::constructor',
          },
          'Initial CRDT state is not a valid LoroDoc snapshot — starting empty and will seed from fallback text'
        );
      }
    }
  }

  getDoc(): LoroDoc { return this.doc; }

  /** Whether the LoroDoc was loaded from saved CRDT state */
  hasContent(): boolean {
    return this.loadedFromCrdt;
  }

  startSync(
    local: LocalDatabaseService,
    remote: RemoteDatabaseService,
    recordId: string,
    sessionId: string,
    debounceMs: number,
  ): void {
    this.local = local;
    this.remote = remote;
    this.recordId = recordId;
    this.sessionId = sessionId;
    this.remoteDebounceMs = debounceMs;
    // Every local Loro update writes the snapshot to the local cache
    // *immediately* (so reload/offline see the latest text), then
    // schedules a debounced push to remote. The local UPSERT is cheap —
    // it's an in-memory SurrealKV write — but errors are swallowed so a
    // bad write never blocks user input.
    this.unsubscribe = this.doc.subscribeLocalUpdates(() => {
      void this.persistLocal();
      this.scheduleRemotePush();
    });
  }

  stopSync(): void {
    if (this.unsubscribe) { this.unsubscribe(); this.unsubscribe = null; }
    if (this.pushTimer) { clearTimeout(this.pushTimer); this.pushTimer = null; }
    if (this.remote && this.recordId) { void this.pushToRemote(); }
  }

  importRemote(state: Uint8Array): void {
    // Echo suppression: skip imports within `remoteDebounceMs + 200` of
    // our own push. The +200 guards against round-trip jitter where our
    // own write echoes back from the LIVE feed before the debounce
    // window closes.
    if (Date.now() - this.lastPushTime < this.remoteDebounceMs + 200) return;
    try {
      this.doc.import(state);
      // Persist the merged snapshot locally so the next reload/offline
      // open sees the freshest converged state without waiting for the
      // LIVE feed.
      void this.persistLocal();
    } catch (e) {
      this.logger?.warn(
        { error: e, Category: 'sp00ky-client::CrdtField::importRemote' },
        'Failed to import remote CRDT state'
      );
    }
  }

  exportSnapshot(): Uint8Array {
    return this.doc.export({ mode: 'snapshot' });
  }

  /** Push this session's cursor blob into the parent row at
   *  `<field>.cursors[$sid]`. No-op when cursors aren't enabled on this
   *  field — the editor still calls this method optimistically, but
   *  without `@cursor` on the schema there's nowhere to store the blob.
   *  The UPDATE itself fires the parent table's LIVE feed, so other
   *  browsers receive the cursor change without a separate `_00_rv` bump. */
  async pushCursorState(encoded: Uint8Array): Promise<void> {
    if (!this.remote || !this.recordId) return;
    if (!this.cursorsEnabled) return;
    this.lastCursorPushTime = Date.now();
    try {
      const state = encodeBase64(encoded);
      await this.remote.query(
        `UPDATE $id SET ${this.fieldName}.cursors[$sid] = $state RETURN NONE;`,
        {
          id: parseRecordIdString(this.recordId),
          sid: this.sessionId,
          state,
        }
      );
    } catch (e) {
      this.logger?.warn(
        { error: e, Category: 'sp00ky-client::CrdtField::pushCursorState' },
        'Failed to push cursor state'
      );
    }
  }

  /** Import remote cursor state (called by CrdtManager from LIVE SELECT) */
  importRemoteCursor(base64State: string): void {
    if (Date.now() - this.lastCursorPushTime < 300) return; // echo suppression
    try {
      const data = decodeBase64(base64State);
      if (this._onCursorUpdate) {
        this._onCursorUpdate(data);
      } else {
        // Only keep the latest cursor state — older positions are useless
        this.pendingCursorUpdate = data;
      }
    } catch (e) {
      this.logger?.warn(
        { error: e, Category: 'sp00ky-client::CrdtField::importRemoteCursor' },
        'Failed to apply remote cursor data'
      );
    }
  }

  private scheduleRemotePush(): void {
    if (this.pushTimer) clearTimeout(this.pushTimer);
    this.pushTimer = setTimeout(() => void this.pushToRemote(), this.remoteDebounceMs);
  }

  /** SET path inside a parent row for the current snapshot. `@crdt`-only
   *  fields hold the snapshot directly (`<field>`); `@crdt @cursor`
   *  fields hold a `{ state, cursors }` object so the snapshot lives at
   *  `<field>.state` next to per-session cursor blobs. */
  private statePath(): string {
    return this.cursorsEnabled ? `${this.fieldName}.state` : this.fieldName;
  }

  /** Mirror the LoroDoc snapshot into the parent row locally. Runs on
   *  every local update and every remote import so reloads (online or
   *  offline) see the freshest content immediately. Failures are
   *  swallowed — a stale local write must never block user input. */
  private async persistLocal(): Promise<void> {
    if (!this.local || !this.recordId) return;
    try {
      await this.local.query(
        `UPDATE $id SET ${this.statePath()} = $state RETURN NONE;`,
        { id: parseRecordIdString(this.recordId), state: this.exportSnapshot() }
      );
    } catch (e) {
      this.logger?.debug(
        { error: e, Category: 'sp00ky-client::CrdtField::persistLocal' },
        'Local CRDT persist failed (best-effort)'
      );
    }
  }

  private async pushToRemote(): Promise<void> {
    if (!this.remote || !this.recordId) return;
    this.lastPushTime = Date.now();
    try {
      // The UPDATE on the parent fires the parent table's LIVE feed,
      // so cross-browser receivers see this change directly in the LIVE
      // payload — no separate `_00_rv` bump or sidecar UPSERT.
      await this.remote.query(
        `UPDATE $id SET ${this.statePath()} = $state RETURN NONE;`,
        { id: parseRecordIdString(this.recordId), state: this.exportSnapshot() }
      );
      this.pushRetryCount = 0;
    } catch (e) {
      this.logger?.warn(
        { error: e, Category: 'sp00ky-client::CrdtField::pushToRemote' },
        'Failed to push CRDT state to remote'
      );
      // Bounded retry. Offline first-loads will exhaust this and stop
      // hammering; the next user keystroke (or the next time a remote
      // event lands once we're back online) will kick off another push.
      if (this.pushRetryCount < 2) {
        this.pushRetryCount++;
        this.scheduleRemotePush();
      }
    }
  }
}

export function decodeBase64(b64: string): Uint8Array {
  if (typeof atob === 'function') {
    const binary = atob(b64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return bytes;
  }
  return new Uint8Array(Buffer.from(b64, 'base64'));
}

export function encodeBase64(bytes: Uint8Array): string {
  if (typeof btoa === 'function') {
    let binary = '';
    for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
    return btoa(binary);
  }
  return Buffer.from(bytes).toString('base64');
}
