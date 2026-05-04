import { LoroDoc } from 'loro-crdt';
import type { RemoteDatabaseService } from '../../services/database/index';
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
  private remote: RemoteDatabaseService | null = null;
  private recordId: string | null = null;
  private sessionId: string = '';
  private unsubscribe: (() => void) | null = null;
  private lastPushTime = 0;
  private lastCursorPushTime = 0;
  private loadedFromCrdt = false;
  private pushRetryCount = 0;
  private logger: Logger | null;

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

  startSync(remote: RemoteDatabaseService, recordId: string, sessionId: string): void {
    this.remote = remote;
    this.recordId = recordId;
    this.sessionId = sessionId;
    this.unsubscribe = this.doc.subscribeLocalUpdates(() => {
      this.schedulePush();
    });
  }

  stopSync(): void {
    if (this.unsubscribe) { this.unsubscribe(); this.unsubscribe = null; }
    if (this.pushTimer) { clearTimeout(this.pushTimer); this.pushTimer = null; }
    if (this.remote && this.recordId) { void this.pushToRemote(); }
  }

  importRemote(base64State: string): void {
    // Suppress echo: skip imports within 500ms of our own push
    if (Date.now() - this.lastPushTime < 500) return;
    try {
      this.doc.import(decodeBase64(base64State));
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

  private schedulePush(): void {
    if (this.pushTimer) clearTimeout(this.pushTimer);
    this.pushTimer = setTimeout(() => void this.pushToRemote(), 300);
  }

  private async pushToRemote(): Promise<void> {
    if (!this.remote || !this.recordId) return;
    this.lastPushTime = Date.now();
    try {
      // Write the CRDT snapshot into the `_00_crdt` table keyed by the
      // array record id `[record_id, field]`, then bump the parent's
      // `_00_rv` so its LIVE feed fires for every viewer that can SELECT
      // it. The bump is what triggers cross-browser delivery now that the
      // snapshot is no longer embedded on the parent row.
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
      if (this.pushRetryCount < 2) {
        this.pushRetryCount++;
        this.schedulePush();
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
