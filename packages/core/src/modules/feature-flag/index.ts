import type { SchemaStructure } from '@spooky-sync/query-builder';
import type { DataModule } from '../data/index';
import type { Sp00kySync } from '../sync/index';
import type { AuthService } from '../auth/index';
import type { Logger } from '../../services/logger/index';
import type { QueryTimeToLive } from '../../types';

const FEATURE_QUERY =
  'SELECT key, variant, payload FROM _00_user_feature WHERE key = $key';

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

interface ActiveHandle {
  handle: FeatureFlagHandle;
  ttl: QueryTimeToLive;
}

export class FeatureFlagModule<S extends SchemaStructure> {
  private logger: Logger;
  private active = new Set<ActiveHandle>();
  private authUnsubscribe: (() => void) | null = null;
  private lastUserId: string | null = null;

  constructor(private deps: FeatureFlagModuleDeps<S>) {
    this.logger = deps.logger.child({ service: 'FeatureFlagModule' });
  }

  init(): void {
    if (this.authUnsubscribe) return;
    this.authUnsubscribe = this.deps.auth.subscribe((userId) => {
      if (userId === this.lastUserId) return;
      this.lastUserId = userId;
      void this.refreshAll();
    });
  }

  feature(key: string, options: FeatureFlagOptions = {}): FeatureFlagHandle {
    const handle = new FeatureFlagHandle(key, options.fallback);
    const entry: ActiveHandle = { handle, ttl: options.ttl ?? '10m' };
    this.active.add(entry);
    handle.onClose(() => this.active.delete(entry));
    void this.register(entry);
    return handle;
  }

  async closeAll(): Promise<void> {
    this.authUnsubscribe?.();
    this.authUnsubscribe = null;
    for (const entry of [...this.active]) entry.handle.close();
  }

  private async refreshAll(): Promise<void> {
    for (const entry of this.active) {
      entry.handle.detach();
      entry.handle.set({ variant: undefined, payload: undefined });
      await this.register(entry);
    }
  }

  private async register(entry: ActiveHandle): Promise<void> {
    const { handle, ttl } = entry;
    try {
      const hash = await this.deps.dataModule.query(
        '_00_user_feature' as any,
        FEATURE_QUERY,
        { key: handle.key },
        ttl,
      );

      this.deps.sync.enqueueDownEvent({ type: 'register', payload: { hash } });

      const unsub = this.deps.dataModule.subscribe(
        hash,
        (records) => {
          const row = records[0] as { variant?: string; payload?: unknown } | undefined;
          handle.set({ variant: row?.variant, payload: row?.payload });
        },
        { immediate: true },
      );

      handle.attach(unsub);
    } catch (err) {
      this.logger.warn(
        { err, key: handle.key, Category: 'sp00ky-client::FeatureFlagModule::register' },
        'Failed to register feature flag query',
      );
    }
  }
}
