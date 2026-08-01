import type { JSX } from 'solid-js';
import {
  createSignal,
  onMount,
  onCleanup,
  createComponent,
  createMemo,
  mergeProps,
} from 'solid-js';
import type { SchemaStructure } from '@spooky/query-builder';
import type { SyncedDbConfig } from '../types';
import { SyncedDb } from '../index';
import { Sp00kyContext } from './context';

export interface Sp00kyProviderProps<S extends SchemaStructure> {
  config: SyncedDbConfig<S>;
  fallback?: JSX.Element;
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
  children: JSX.Element;
}

export function Sp00kyProvider<S extends SchemaStructure>(
  props: Sp00kyProviderProps<S>
): JSX.Element {
  const merged = mergeProps(
    {
      fallback: undefined as JSX.Element | undefined,
    },
    props
  );

  const [db, setDb] = createSignal<SyncedDb<S> | undefined>(undefined);

  // `onMount` is async, so a dispose can land mid-init. Only that narrow race is
  // handled here: an instance whose init finished AFTER the provider was already
  // gone is closed, because nothing will ever reference it.
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

  onMount(async () => {
    try {
      const instance = new SyncedDb<S>(merged.config);
      await instance.init();
      if (disposed) {
        await instance.close();
        return;
      }
      // Gate first-load UI on prewarmed data. Best-effort: never let a preload
      // failure keep the app stuck on the fallback.
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
  });

  const content = createMemo(() => {
    const instance = db();
    if (!instance) return merged.fallback;
    return createComponent(Sp00kyContext.Provider, {
      value: instance,
      get children() {
        return merged.children;
      },
    });
  });

  return content as unknown as JSX.Element;
}
