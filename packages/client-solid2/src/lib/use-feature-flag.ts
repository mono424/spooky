import type { Accessor } from 'solid-js';
import { onCleanup } from 'solid-js';
import { useDb } from './context';
import { fromSubscription } from './from-subscription';
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
 * the same SSP + sync pipeline that powers `createQuery`, so toggling a flag
 * via `spky flag enable <key>` propagates to the UI without a refresh.
 *
 * `enabled()` is `true` when the resolved variant exists and is not 'off'.
 * For multi-variant flags, prefer `variant()` directly.
 */
export function useFeatureFlag(key: string, options?: FeatureFlagOptions): UseFeatureFlag {
  const db = useDb();
  const handle = db.getSp00ky().feature(key, options);
  onCleanup(() => handle.close());

  const state = fromSubscription<{ variant: string | undefined; payload: unknown }>(
    (cb) =>
      handle.subscribe((s) => cb({ variant: s.variant ?? options?.fallback, payload: s.payload })),
    { variant: handle.variant(), payload: handle.payload() }
  );

  return {
    variant: () => state().variant,
    payload: () => state().payload,
    enabled: () => {
      const v = state().variant;
      return v !== undefined && v !== 'off';
    },
  };
}
