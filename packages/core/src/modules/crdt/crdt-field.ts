import { LoroDoc } from 'loro-crdt';
import type { RemoteDatabaseService } from '../../services/database/index';
import { parseRecordIdString } from '../../utils/index';

export class CrdtField {
  private doc: LoroDoc;
  private pushTimer: ReturnType<typeof setTimeout> | null = null;
  private remote: RemoteDatabaseService | null = null;
  private recordId: string | null = null;
  private unsubscribe: (() => void) | null = null;
  /** Timestamp of the last push — imports within 500ms are suppressed (echo) */
  private lastPushTime = 0;

  private loadedFromCrdt = false;

  constructor(private fieldName: string, initialState?: string, _fallbackText?: string) {
    this.doc = new LoroDoc();
    if (initialState) {
      this.doc.import(decodeBase64(initialState));
      this.loadedFromCrdt = true;
    }
    // fallbackText is NOT loaded into the LoroDoc here —
    // the editor handles it via the content prop + reconfigure() pattern
  }

  getDoc(): LoroDoc { return this.doc; }

  /** Whether the LoroDoc was loaded from saved CRDT state */
  hasContent(): boolean {
    return this.loadedFromCrdt;
  }

  startSync(remote: RemoteDatabaseService, recordId: string): void {
    this.remote = remote;
    this.recordId = recordId;
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
    try { this.doc.import(decodeBase64(base64State)); } catch {}
  }

  exportSnapshot(): string {
    return encodeBase64(this.doc.export({ mode: 'snapshot' }));
  }

  private schedulePush(): void {
    if (this.pushTimer) clearTimeout(this.pushTimer);
    this.pushTimer = setTimeout(() => void this.pushToRemote(), 300);
  }

  private async pushToRemote(): Promise<void> {
    if (!this.remote || !this.recordId) return;
    this.lastPushTime = Date.now();
    try {
      await this.remote.query(
        `INSERT INTO _00_crdt (record_id, field, state) VALUES ($rid, $field, $state)
         ON DUPLICATE KEY UPDATE state = $state`,
        { rid: parseRecordIdString(this.recordId), field: this.fieldName, state: this.exportSnapshot() }
      );
    } catch {}
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
