import { createSignal, onMount, createComponent, createMemo, JSX, mergeProps } from 'solid-js';
import type { SchemaStructure } from '@spooky/query-builder';
import type { SyncedDbConfig } from '../types';
import { SyncedDb } from '../index';
import { Sp00kyContext } from './context';

export interface Sp00kyProviderProps<S extends SchemaStructure> {
  config: SyncedDbConfig<S>;
  fallback?: JSX.Element;
  onError?: (error: Error) => void;
  onReady?: (db: SyncedDb<S>) => void;
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

  onMount(async () => {
    try {
      const instance = new SyncedDb<S>(merged.config);
      await instance.init();
      setDb(() => instance);
      merged.onReady?.(instance);
    } catch (e) {
      const error = e instanceof Error ? e : new Error(String(e));
      if (merged.onError) {
        merged.onError(error);
      } else {
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
