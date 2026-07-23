import { createSignal, onCleanup, type Accessor } from 'solid-js';
import { useDb } from './context';
import { semverGt, type AppReleaseOptions, type AppReleaseSnapshot } from '@spooky-sync/core';

export interface UseAppReleaseOptions extends AppReleaseOptions {
  /** App name from sp00ky.yml, e.g. `web`. */
  app: string;
  /**
   * The running build's version (X.Y.Z), typically baked in at build time
   * (e.g. a vite `define` from package.json). `updateAvailable()` is true when
   * the announced release is semver-newer than this.
   */
  currentVersion: string;
}

export interface UseAppRelease {
  /** Latest announced version for the app, or undefined when no row exists. */
  latestVersion: Accessor<string | undefined>;
  /** Announced version is semver-newer than the running build. */
  updateAvailable: Accessor<boolean>;
  /** The newer release asks clients to update/reload without prompting. */
  mandatory: Accessor<boolean>;
  /** The newer release asks reloads to clear service-worker caches first. */
  cacheBust: Accessor<boolean>;
  /**
   * Reload onto the announced release. Plain `location.reload()` normally;
   * when the release is flagged cache-bust, CacheStorage is cleared, the
   * service-worker registration is nudged to update, and navigation carries a
   * `?cb=` token to punch through intermediary caches. The service worker is
   * deliberately NOT unregistered: navigating while still controlled by a
   * just-unregistered worker strands subresource fetches on the dead worker
   * and the page hangs until a manual reload.
   */
  reload: () => Promise<void>;
}

async function reloadForSnapshot(snapshot: AppReleaseSnapshot): Promise<void> {
  if (typeof window === 'undefined') return;
  if (snapshot.cacheBust) {
    try {
      if (window.caches) {
        const keys = await window.caches.keys();
        await Promise.all(keys.map((k) => window.caches.delete(k)));
      }
      if (navigator.serviceWorker) {
        const regs = await navigator.serviceWorker.getRegistrations();
        for (const r of regs) r.update().catch(() => {});
      }
      window.location.href = window.location.pathname + '?cb=' + Date.now();
      return;
    } catch {
      /* fall through to a plain reload */
    }
  }
  window.location.reload();
}

/**
 * Observe the app's announced release (`_00_app_release:<app>`, written by
 * `spky deploy` / `spky release`) and compare it against the running build.
 *
 * Typical use: mount a small "new version available — Reload" notification
 * gated on `updateAvailable()`, auto-invoking `reload()` when `mandatory()`
 * (guard the auto path against reload loops with a per-version marker, since
 * a client can reload while the deploy is still rolling out and land on the
 * old bundle again).
 */
export function useAppRelease(options: UseAppReleaseOptions): UseAppRelease {
  const db = useDb();
  const handle = db.getSp00ky().appRelease(options.app, { ttl: options.ttl });

  const [snapshot, setSnapshot] = createSignal<AppReleaseSnapshot>(handle.snapshot());
  const unsub = handle.subscribe(setSnapshot);

  onCleanup(() => {
    unsub();
    handle.close();
  });

  const updateAvailable = () => semverGt(snapshot().version, options.currentVersion);

  return {
    latestVersion: () => snapshot().version,
    updateAvailable,
    mandatory: () => updateAvailable() && snapshot().mandatory,
    cacheBust: () => snapshot().cacheBust,
    reload: () => reloadForSnapshot(snapshot()),
  };
}
