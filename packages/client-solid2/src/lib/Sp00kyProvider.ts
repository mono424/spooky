import type { Element } from 'solid-js';
import {
  createSignal,
  onSettled,
  onCleanup,
  createComponent,
  createMemo,
  merge,
} from 'solid-js';
import type { SchemaStructure } from '@spooky-sync/query-builder';
import type { SyncedDbConfig } from '../types';
import { SyncedDb } from '../index';
import { Sp00kyContext } from './context';

export interface Sp00kyProviderProps<S extends SchemaStructure> {
  config: SyncedDbConfig<S>;
  fallback?: Element;
  onError?: (error: Error) => void;
  onReady?: (db: SyncedDb<S>) => void;
  /**
   * Prewarm data into the local cache before revealing the UI. Runs after
   * `init()`; the `fallback` stays visible until it resolves. Use awaitable
   * `db.preload(...)` calls here to gate first-load on essential data (e.g.
   * config). On warm loads preload returns instantly, so there's no perceptible
   * gate after the first run. Best-effort: a rejection is caught and the UI is
   * revealed anyway.
   */
  preload?: (db: SyncedDb<S>) => Promise<void>;
  children: Element;
}

export function Sp00kyProvider<S extends SchemaStructure>(
  props: Sp00kyProviderProps<S>
): Element {
  const merged = merge({ fallback: undefined as Element | undefined }, props);

  // Written from the async init continuation — outside any tracking scope.
  const [db, setDb] = createSignal<SyncedDb<S> | undefined>(undefined, { ownedWrite: true });

  // Init is async, so a dispose can land mid-init. Only that narrow race is
  // handled here: an instance whose init finished AFTER the provider was
  // already gone is closed, because nothing will ever reference it.
  //
  // A live, mounted client is deliberately NOT closed on cleanup. Doing that
  // nulls `SyncedDb.sp00ky`, so every later `create`/`update`/`delete` throws
  // "SyncedDb not initialized" while reads keep rendering from state that is
  // already subscribed — i.e. mutations die silently and the app looks fine. In
  // a host app the provider wraps the whole tree and only unmounts with the
  // page, where the browser reclaims the worker anyway, so the leak this was
  // meant to fix is worth far less than that risk.
  let disposed = false;

  onCleanup(() => {
    disposed = true;
  });

  // `onSettled` replaces Solid 1's `onMount`.
  onSettled(() => {
    void (async () => {
      try {
        const instance = new SyncedDb<S>(merged.config);
        await instance.init();
        if (disposed) {
          await instance.close();
          return;
        }
        // Gate first-load UI on prewarmed data. Best-effort: never let a
        // preload failure keep the app stuck on the fallback.
        if (merged.preload) {
          try {
            await merged.preload(instance);
          } catch (e) {
            // oxlint-disable-next-line no-console
            console.error('Sp00kyProvider: preload failed; revealing UI anyway', e);
          }
        }
        setDb(() => instance);
        merged.onReady?.(instance);
      } catch (e) {
        const error = e instanceof Error ? e : new Error(String(e));
        if (merged.onError) {
          merged.onError(error);
        } else {
          // oxlint-disable-next-line no-console
          console.error('Sp00kyProvider: Failed to initialize database', error);
        }
      }
    })();
  });

  const content = createMemo(() => {
    const instance = db();
    if (!instance) return merged.fallback;
    // Solid 2: the context object IS the provider component.
    return createComponent(Sp00kyContext, {
      value: instance,
      get children() {
        return merged.children;
      },
    });
  });

  return content as unknown as Element;
}
