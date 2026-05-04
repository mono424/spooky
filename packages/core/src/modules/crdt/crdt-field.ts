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
    initialState?: string,
    logger?: Logger | null,
  ) {
    this.logger = logger ?? null;
    this.doc = new LoroDoc();
    if (initialState) {
      this.doc.import(decodeBase64(initialState));
      this.loadedFromCrdt = true;
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

  importRemote(base64State: string): void {
    // Echo suppression: skip imports within `remoteDebounceMs + 200` of
    // our own push. The +200 guards against round-trip jitter where our
    // own write echoes back from the LIVE feed before the debounce
    // window closes.
    if (Date.now() - this.lastPushTime < this.remoteDebounceMs + 200) return;
    try {
      this.doc.import(decodeBase64(base64State));
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

  exportSnapshot(): string {
    return encodeBase64(this.doc.export({ mode: 'snapshot' }));
  }

  /** Push cursor presence into the `_00_cursor` table, keyed by the array
   *  record id `[record_id, session_id, field]`. The composite id is the
   *  primary key, so UPSERT can target it directly and create-or-update
   *  without a WHERE clause (UPSERT … WHERE silently fails to create rows
   *  when no match exists). All three axes are required: a single browser
   *  session has its own cursor for every open CrdtField, and the blob
   *  itself is bound to one specific LoroDoc, so the receiver needs the
   *  field axis to dispatch each blob to the matching editor. The trailing
   *  `UPDATE $id SET _00_rv = _00_rv` bumps the parent so its LIVE feed
   *  fans the change to other browsers — see CrdtManager.dispatchRow. */
  async pushCursorState(encoded: Uint8Array): Promise<void> {
    if (!this.remote || !this.recordId) return;
    this.lastCursorPushTime = Date.now();
    try {
      const state = encodeBase64(encoded);
      await this.remote.query(
        `UPSERT type::record("_00_cursor", [$id, $sid, $field]) SET record_id = $id, session_id = $sid, field = $field, state = $state;
         UPDATE $id SET _00_rv = _00_rv RETURN NONE;`,
        {
          id: parseRecordIdString(this.recordId),
          sid: this.sessionId,
          field: this.fieldName,
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

  /** Mirror the LoroDoc snapshot into the parent row's local `_00_crdt`
   *  object field. Runs synchronously on every local update and on every
   *  remote import, so a reload (online or offline) sees the freshest
   *  content immediately. The field is injected on every CRDT-bearing
   *  parent by `apps/cli/src/main.rs` for client output only — server
   *  uses a separate `_00_crdt` table. Failures are logged and swallowed:
   *  a stale local write must never block user input. */
  private async persistLocal(): Promise<void> {
    if (!this.local || !this.recordId) return;
    try {
      await this.local.query(
        `UPDATE $id SET _00_crdt[$field] = $state RETURN NONE;`,
        { id: parseRecordIdString(this.recordId), field: this.fieldName, state: this.exportSnapshot() }
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
      // Write the CRDT snapshot into the remote `_00_crdt` table keyed
      // by the composite `[record_id, field]` id, then bump the parent's
      // `_00_rv` so its LIVE feed fires for every viewer that can SELECT
      // it. The bump is what triggers cross-browser delivery — the
      // editor itself is local-only.
      await this.remote.query(
        `UPSERT type::record("_00_crdt", [$id, $field]) SET record_id = $id, field = $field, state = $state;
         UPDATE $id SET _00_rv = _00_rv RETURN NONE;`,
        { id: parseRecordIdString(this.recordId), field: this.fieldName, state: this.exportSnapshot() }
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
