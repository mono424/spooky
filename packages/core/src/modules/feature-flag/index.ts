import type { SchemaStructure } from '@spooky-sync/query-builder';
import type { DataModule } from '../data/index';
import type { Sp00kySync } from '../sync/index';
import type { AuthService } from '../auth/index';
import type { Logger } from '../../services/logger/index';
import type { QueryTimeToLive } from '../../types';

// One shared LIVE query over ALL of the signed-in user's assignments — the
// `_00_user_feature` select permission scopes it to `user = $auth.id`, so no
// per-key `WHERE key = $key` param is needed. A single registration means every
// flag the user is (or becomes) assigned is observed at once: new assignments
// stream in live, and a handle for an unassigned key simply resolves to its
// fallback. Avoids one-registration-per-flag and the param-filtered live query.
const FEATURE_QUERY = 'SELECT key, variant, payload FROM _00_user_feature';

interface FeatureRow {
  key?: string;
  variant?: string;
  payload?: unknown;
}

export interface FeatureFlagSnapshot {
  variant: string | undefined;
  payload: unknown | undefined;
}

export interface FeatureFlagOptions {
  fallback?: string;
  ttl?: QueryTimeToLive;
}

export class FeatureFlagHandle {
  private latest: FeatureFlagSnapshot = { variant: undefined, payload: undefined };
  private listeners = new Set<(s: FeatureFlagSnapshot) => void>();
  private unsubscribeFn: (() => void) | null = null;
  private onCloseFn: (() => void) | null = null;
  private closed = false;

  constructor(
    public readonly key: string,
    public readonly fallback: string | undefined,
  ) {}

  attach(unsubscribe: () => void): void {
    this.unsubscribeFn?.();
    this.unsubscribeFn = unsubscribe;
  }

  detach(): void {
    this.unsubscribeFn?.();
    this.unsubscribeFn = null;
  }

  set(snapshot: FeatureFlagSnapshot): void {
    if (this.closed) return;
    this.latest = snapshot;
    for (const cb of this.listeners) cb(snapshot);
  }

  variant(): string | undefined {
    return this.latest.variant ?? this.fallback;
  }

  payload<T = unknown>(): T | undefined {
    return this.latest.payload as T | undefined;
  }

  enabled(): boolean {
    const v = this.variant();
    return v !== undefined && v !== 'off';
  }

  subscribe(cb: (s: FeatureFlagSnapshot) => void): () => void {
    this.listeners.add(cb);
    cb({ variant: this.variant(), payload: this.latest.payload });
    return () => {
      this.listeners.delete(cb);
    };
  }

  onClose(cb: () => void): void {
    this.onCloseFn = cb;
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    this.listeners.clear();
    this.detach();
    this.onCloseFn?.();
  }
}

export interface FeatureFlagModuleDeps<S extends SchemaStructure> {
  dataModule: DataModule<S>;
  sync: Sp00kySync<S>;
  auth: AuthService<S>;
  logger: Logger;
}

export class FeatureFlagModule<S extends SchemaStructure> {
  private logger: Logger;
  private handles = new Set<FeatureFlagHandle>();
  private authUnsubscribe: (() => void) | null = null;
  private lastUserId: string | null = null;

  // The single shared live query over the user's assignments.
  private querySubscription: (() => void) | null = null;
  private starting = false;
  // Longest TTL any caller asked for (the query is shared across all flags).
  private ttl: QueryTimeToLive = '10m';
  // Latest assignment per key, plus whether the query has resolved at least
  // once (so a handle created before the first result knows to wait vs. fall
  // back). `snapshots` only holds ASSIGNED keys; an absent key → fallback.
  private snapshots = new Map<string, FeatureFlagSnapshot>();
  private loaded = false;

  constructor(private deps: FeatureFlagModuleDeps<S>) {
    this.logger = deps.logger.child({ service: 'FeatureFlagModule' });
  }

  init(): void {
    if (this.authUnsubscribe) return;
    this.authUnsubscribe = this.deps.auth.subscribe((userId) => {
      if (userId === this.lastUserId) return;
      this.lastUserId = userId;
      void this.refresh();
    });
  }

  feature(key: string, options: FeatureFlagOptions = {}): FeatureFlagHandle {
    const handle = new FeatureFlagHandle(key, options.fallback);
    this.handles.add(handle);
    handle.onClose(() => this.handles.delete(handle));
    if (options.ttl) this.ttl = options.ttl;
    // If the shared query already resolved, seed this handle immediately so a
    // late `feature()` call doesn't flash the fallback for an assigned key.
    if (this.loaded) {
      handle.set(this.snapshots.get(key) ?? { variant: undefined, payload: undefined });
    }
    void this.ensureStarted();
    return handle;
  }

  async closeAll(): Promise<void> {
    this.authUnsubscribe?.();
    this.authUnsubscribe = null;
    this.teardownQuery();
    for (const handle of [...this.handles]) handle.close();
  }

  /** Auth changed: drop the old user's query/snapshots and re-observe. */
  private async refresh(): Promise<void> {
    this.teardownQuery();
    this.loaded = false;
    this.snapshots.clear();
    // Clear handles immediately so a sign-out hides flag-gated UI without lag.
    for (const handle of this.handles) {
      handle.set({ variant: undefined, payload: undefined });
    }
    await this.ensureStarted();
  }

  private teardownQuery(): void {
    this.querySubscription?.();
    this.querySubscription = null;
  }

  /** Start the single shared live query (idempotent; no-op with no handles). */
  private async ensureStarted(): Promise<void> {
    if (this.querySubscription || this.starting || this.handles.size === 0) return;
    this.starting = true;
    try {
      const hash = await this.deps.dataModule.query(
        '_00_user_feature' as any,
        FEATURE_QUERY,
        {},
        this.ttl,
      );
      this.deps.sync.enqueueDownEvent({ type: 'register', payload: { hash } });
      this.querySubscription = this.deps.dataModule.subscribe(
        hash,
        (records) => this.applyRecords(records as FeatureRow[]),
        { immediate: true },
      );
    } catch (err) {
      this.logger.warn(
        { err, Category: 'sp00ky-client::FeatureFlagModule::register' },
        'Failed to register feature flag query',
      );
    } finally {
      this.starting = false;
    }
  }

  /** Live query result → per-key snapshots → push to every active handle. */
  private applyRecords(records: FeatureRow[]): void {
    this.snapshots.clear();
    for (const row of records ?? []) {
      if (row && typeof row.key === 'string') {
        this.snapshots.set(row.key, { variant: row.variant, payload: row.payload });
      }
    }
    this.loaded = true;
    for (const handle of this.handles) {
      handle.set(this.snapshots.get(handle.key) ?? { variant: undefined, payload: undefined });
    }
  }
}
