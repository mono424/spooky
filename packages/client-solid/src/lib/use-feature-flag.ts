import { createSignal, onCleanup, type Accessor } from 'solid-js';
import { useDb } from './context';
import type { FeatureFlagOptions } from '@spooky-sync/core';

export interface UseFeatureFlag {
  variant: Accessor<string | undefined>;
  payload: Accessor<unknown | undefined>;
  enabled: Accessor<boolean>;
}

/**
 * Subscribe to a feature flag for the currently authenticated user.
 *
 * Returns three Solid accessors that update reactively whenever the
 * server-materialized assignment in `_00_user_feature` changes. Backed by
 * the same SSP + sync pipeline that powers `useQuery`, so toggling a flag
 * via `spky flag enable <key>` propagates to the UI without a refresh.
 *
 * `enabled()` is `true` when the resolved variant exists and is not 'off'.
 * For multi-variant flags, prefer `variant()` directly.
 */
export function useFeatureFlag(
  key: string,
  options?: FeatureFlagOptions,
): UseFeatureFlag {
  const db = useDb();
  const handle = db.getSp00ky().feature(key, options);

  const [variant, setVariant] = createSignal<string | undefined>(handle.variant());
  const [payload, setPayload] = createSignal<unknown | undefined>(handle.payload());

  const unsub = handle.subscribe((s) => {
    setVariant(s.variant ?? options?.fallback);
    setPayload(s.payload);
  });

  onCleanup(() => {
    unsub();
    handle.close();
  });

  return {
    variant,
    payload,
    enabled: () => {
      const v = variant();
      return v !== undefined && v !== 'off';
    },
  };
}
