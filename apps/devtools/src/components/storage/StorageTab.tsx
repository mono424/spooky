import { Show, For, createEffect, createMemo, createSignal } from 'solid-js';
import { useDevTools } from '../../context/DevToolsContext';
import { formatBytes, formatMs } from '../../utils/formatters';
import { JsonView } from '../ui/JsonView';
import type { OpfsEntry } from '../../types/devtools';

/** Filter the serializer's legacy `'undefined'` string (older page scripts). */
function realError(err: string | undefined): string | undefined {
  return err && err !== 'undefined' ? err : undefined;
}

export function StorageTab() {
  const {
    state,
    storageInfo,
    storageInfoError,
    isFetchingStorage,
    fetchStorageInfo,
    requestPersistentStorage,
    isSp00kyAvailable,
  } = useDevTools();

  // Grant/deny outcome of the "Request persistent storage" button.
  const [persistResult, setPersistResult] = createSignal<boolean | null>(null);

  // Fetch once Sp00ky is detected (the panel can mount before the page's
  // client is up — same gating as DatabaseTab's table enumeration).
  createEffect(() => {
    if (!isSp00kyAvailable()) return;
    if (!storageInfo() && !isFetchingStorage()) void fetchStorageInfo();
  });

  // Live health beats snapshot health: `state.database.storage` updates via the
  // push channel on every change; the snapshot only on refresh.
  const health = createMemo(
    (): { status: string; fallback: boolean; error?: string; role?: string } =>
      state.database.storage ?? storageInfo()?.health ?? { status: 'unknown', fallback: false }
  );

  const info = storageInfo;
  const diag = () => info()?.engineDiagnostics;
  const stats = () => info()?.sqliteStats as Record<string, any> | undefined;

  // Shared-tabs: live push state beats the snapshot (same reason as health).
  // Present only when the app configured `sharedTabs: true`.
  const tabs = createMemo(() => state.database.tabs ?? info()?.tabs ?? null);
  const role = () => (tabs()?.active ? tabs()!.role : undefined) ?? health().role;
  const isLeader = () => role() === 'leader';
  const isFollower = () => role() === 'follower';

  /** What the health status means for THIS tab, given its shared-tabs role. */
  const healthExplanation = (): string => {
    if (health().status === 'persistent') {
      if (isFollower()) {
        return 'Durable: this tab shares the leader tab\'s OPFS store over a MessagePort.';
      }
      if (isLeader()) {
        return 'Durable: this tab owns the OPFS worker and serves the other tabs.';
      }
      return 'Local store is OPFS-backed and survives reloads.';
    }
    if (health().status === 'memory') {
      return 'In-memory store as configured (store: memory) — nothing persists by design.';
    }
    return 'This engine does not report storage health.';
  };

  /** The fallback hint must not tell you to close other tabs when sharing them
   *  is exactly what the app enabled. */
  const fallbackHint = (): string => {
    if (tabs()?.active) {
      return isLeader()
        ? 'This tab was elected leader but could not open the OPFS pool, so every tab is now served from RAM. Usually a previous leader\'s worker has not released its file handles yet; a later election retries automatically.'
        : 'Shared-tabs is active but this tab is not attached to a leader\'s store, so it fell back to RAM. It should recover on the next election; if it does not, reload.';
    }
    if (tabs() && !tabs()!.active) {
      return 'sharedTabs was requested but this tab runs alone, so it contends for the OPFS pool with every other tab and lost. Fix the reason above (see the Tabs section) or close other tabs and reload.';
    }
    return 'The whole dataset sits in RAM and every local write is lost on reload. The usual cause is another tab of this app holding the OPFS pool lock — close other tabs and reload.';
  };

  /** Human wording for a capability-gate / fallback reason code. */
  const reasonText = (reason: string | undefined): string => {
    switch (reason) {
      case 'flag-off':
        return 'sharedTabs is not enabled';
      case 'not-browser':
        return 'not a browser context';
      case 'no-shared-worker':
        return 'this browser has no SharedWorker (pre-148 Chrome on Android, some WebViews)';
      case 'no-web-locks':
        return 'this browser has no Web Locks API';
      case 'no-message-channel':
        return 'this browser has no MessageChannel';
      case 'engine-not-sqlite':
        return 'shared-tabs requires localEngine: "sqlite"';
      case 'fell-back':
        return 'the broker assigned no role in time (election timeout, or a tab from a different app build was rejected)';
      default:
        return reason ?? 'unknown';
    }
  };

  const usagePct = createMemo(() => {
    const b = info()?.browser;
    if (!b?.quota || b.usage === undefined) return null;
    return Math.min(100, (b.usage / b.quota) * 100);
  });

  const blobs = () => info()?.blobs;
  const blobBudgetPct = createMemo(() => {
    const b = blobs();
    if (!b?.budgetBytes) return null;
    return Math.min(100, (b.totalBytes / b.budgetBytes) * 100);
  });

  const workerSelectDowngraded = () =>
    !!diag() && diag()!.workerSelectConfigured && !diag()!.workerSelectEffective;

  // Group OPFS entries by top-level directory for the file table.
  const activePoolDir = () => {
    const bucket = info()?.engine.bucketId;
    return bucket ? `.sp00ky-${bucket}` : null;
  };
  const topLevelOf = (e: OpfsEntry) => e.path.split('/')[0];
  const isOrphanPool = (e: OpfsEntry) => {
    const top = topLevelOf(e);
    return top.startsWith('.sp00ky-') && top !== activePoolDir();
  };

  /**
   * A file whose size cannot be read is held by an exclusive sync access
   * handle, i.e. someone has that pool open. In the active pool that is the
   * expected, healthy state; say WHO holds it so a locked file does not read
   * as a problem.
   */
  const lockedFileNote = (entry: OpfsEntry): string => {
    if (topLevelOf(entry) !== activePoolDir()) return 'locked by another context';
    if (isLeader()) return 'locked — held by this tab\'s worker (expected)';
    if (isFollower()) return 'locked — held by the leader tab\'s worker (expected)';
    return 'locked — a worker holds this pool open';
  };

  const handlePersist = async () => {
    setPersistResult(null);
    setPersistResult(await requestPersistentStorage());
  };

  return (
    <div class="mcp-container">
      <div class="mcp-header">
        <h2>Storage</h2>
        <div class="mcp-header-controls">
          <button
            class="storage-refresh-btn"
            disabled={isFetchingStorage()}
            onClick={() => void fetchStorageInfo()}
          >
            {isFetchingStorage() ? 'Refreshing…' : 'Refresh'}
          </button>
        </div>
      </div>

      {/* 1. Health banner — the OPFS pain point, always first. */}
      <Show when={health().fallback}>
        <div class="storage-health-banner error">
          <div class="storage-health-title">Persistence lost — running IN MEMORY</div>
          <Show when={realError(health().error)}>
            <code class="storage-health-reason">{realError(health().error)}</code>
          </Show>
          <div class="storage-health-hint">{fallbackHint()}</div>
        </div>
      </Show>
      <Show when={!health().fallback}>
        <div class="storage-health-line">
          <span
            class="status-pill"
            classList={{
              'status-active': health().status === 'persistent',
              'status-initializing': health().status === 'unknown',
            }}
          >
            <span class="status-dot" />
            {health().status}
            <Show when={role()}>{` · ${role()}`}</Show>
          </span>
          <span class="muted">{healthExplanation()}</span>
        </div>
      </Show>

      <Show
        when={info() || storageInfoError()}
        fallback={
          <div class="empty-state">
            {isSp00kyAvailable() ? 'Loading storage diagnostics…' : 'Waiting for Sp00ky…'}
          </div>
        }
      >
        <Show when={storageInfoError() && !info()}>
          <div class="empty-state">
            Storage diagnostics unavailable: {storageInfoError()}
          </div>
        </Show>

        <Show when={info()}>
          {/* 2. Engine */}
          <div class="mcp-section">
            <h3>Engine</h3>
            <div class="kv">
              <div class="kv-row">
                <span class="kv-k">Engine</span>
                <span class="kv-v">{info()!.engine.kind}</span>
              </div>
              <div class="kv-row">
                <span class="kv-k">Store</span>
                <span class="kv-v">{info()!.engine.store}</span>
              </div>
              <div class="kv-row">
                <span class="kv-k">Bucket</span>
                <span class="kv-v mono">{info()!.engine.bucketId}</span>
              </div>
              <Show when={diag()}>
                <div class="kv-row">
                  <span class="kv-k">OPFS requested</span>
                  <span class="kv-v">{diag()!.useOpfs ? 'yes' : 'no'}</span>
                </div>
                <div class="kv-row">
                  <span class="kv-k">workerSelect</span>
                  <span class="kv-v">
                    <Show
                      when={workerSelectDowngraded()}
                      fallback={diag()!.workerSelectEffective ? 'on' : 'off'}
                    >
                      <span class="status-pill status-initializing">
                        <span class="status-dot" />
                        downgraded at runtime (stale worker script)
                      </span>
                    </Show>
                  </span>
                </div>
                <Show when={realError(diag()!.error)}>
                  <div class="kv-row">
                    <span class="kv-k">Diagnostics error</span>
                    <span class="kv-v storage-error-text">{realError(diag()!.error)}</span>
                  </div>
                </Show>
              </Show>
            </div>
          </div>

          {/* 3. Shared tabs — who owns this store. Only when the app asked. */}
          <Show when={tabs()}>
            <div class="mcp-section">
              <h3>Shared tabs</h3>
              <Show
                when={tabs()!.active}
                fallback={
                  <div class="kv">
                    <div class="kv-row">
                      <span class="kv-k">Status</span>
                      <span class="kv-v">
                        <span class="status-pill status-initializing">
                          <span class="status-dot" />
                          running alone
                        </span>
                      </span>
                    </div>
                    <div class="kv-row">
                      <span class="kv-k">Reason</span>
                      <span class="kv-v">{reasonText(tabs()!.reason)}</span>
                    </div>
                    <div class="kv-row">
                      <span class="kv-k" />
                      <span class="kv-v muted">
                        sharedTabs is configured, so this tab was meant to share one durable store
                        with the others. It does not, so it competes for the OPFS pool: only one tab
                        wins and the rest run in RAM.
                      </span>
                    </div>
                  </div>
                }
              >
                <div class="kv">
                  <div class="kv-row">
                    <span class="kv-k">Role</span>
                    <span class="kv-v">
                      <span
                        class="status-pill"
                        classList={{
                          'status-active': isLeader(),
                          'status-updating': isFollower(),
                          'status-initializing': !isLeader() && !isFollower(),
                        }}
                      >
                        <span class="status-dot" />
                        {tabs()!.role}
                      </span>
                      <span class="muted">
                        {isLeader()
                          ? ' owns the OPFS worker and the sync loop (outbox drain + list_ref LIVE)'
                          : isFollower()
                            ? ' reads/writes the leader\'s store over a MessagePort'
                            : ' between leaders — ops are parked until a role lands'}
                      </span>
                    </span>
                  </div>
                  <div class="kv-row">
                    <span class="kv-k">This tab</span>
                    <span class="kv-v mono">{tabs()!.tabId ?? '—'}</span>
                  </div>
                  <div class="kv-row">
                    <span class="kv-k">Leader</span>
                    <span class="kv-v mono">
                      {tabs()!.leaderTabId ?? '—'}
                      <Show when={isLeader()}>
                        <span class="muted"> (this tab)</span>
                      </Show>
                    </span>
                  </div>
                  <div class="kv-row">
                    <span class="kv-k">Leadership term</span>
                    <span class="kv-v">
                      #{tabs()!.leadershipId ?? 0}
                      <span class="muted"> (rises on every failover; names the worker lock)</span>
                    </span>
                  </div>
                  <Show when={isLeader()}>
                    <div class="kv-row">
                      <span class="kv-k">Followers attached</span>
                      <span class="kv-v">{tabs()!.followers ?? 0}</span>
                    </div>
                    <div class="kv-row">
                      <span class="kv-k">Ingest batches relayed</span>
                      <span class="kv-v">
                        {tabs()!.relayedBatches ?? 0}
                        <span class="muted"> (fan-out that keeps follower queries live)</span>
                      </span>
                    </div>
                  </Show>
                  <Show when={stats()?.roleChanges !== undefined}>
                    <div class="kv-row">
                      <span class="kv-k">Role changes</span>
                      <span class="kv-v">
                        {stats()!.roleChanges}
                        <span class="muted"> (this tab's promotions + attachments)</span>
                      </span>
                    </div>
                  </Show>
                  <Show when={isFollower() && stats()?.proxiedOps !== undefined}>
                    <div class="kv-row">
                      <span class="kv-k">Ops via leader</span>
                      <span class="kv-v">{stats()!.proxiedOps}</span>
                    </div>
                  </Show>
                </div>
              </Show>
            </div>
          </Show>

          {/* 4. Space */}
          <div class="mcp-section">
            <h3>Space</h3>
            <Show when={realError(info()!.browser.error)}>
              <div class="storage-error-text">{realError(info()!.browser.error)}</div>
            </Show>
            <Show when={usagePct() !== null}>
              <div class="storage-usage-bar" title={`${usagePct()!.toFixed(1)}% of quota`}>
                <div class="storage-usage-fill" style={{ width: `${usagePct()}%` }} />
              </div>
            </Show>
            <div class="kv">
              <Show when={info()!.browser.usage !== undefined}>
                <div class="kv-row">
                  <span class="kv-k">Origin usage</span>
                  <span class="kv-v">
                    {formatBytes(info()!.browser.usage)}
                    <Show when={info()!.browser.quota !== undefined}>
                      {' '}of {formatBytes(info()!.browser.quota)} quota
                      {usagePct() !== null ? ` (${usagePct()!.toFixed(1)}%)` : ''}
                    </Show>
                  </span>
                </div>
              </Show>
              <For each={Object.entries(info()!.browser.usageDetails ?? {})}>
                {([system, bytes]) => (
                  <div class="kv-row">
                    <span class="kv-k">— {system}</span>
                    <span class="kv-v">{formatBytes(bytes)}</span>
                  </div>
                )}
              </For>
              <Show when={diag()?.dbSizeBytes !== undefined}>
                <div class="kv-row">
                  <span class="kv-k">SQLite DB size</span>
                  <span class="kv-v">{formatBytes(diag()!.dbSizeBytes)}</span>
                </div>
              </Show>
              <Show when={diag()?.freelistBytes !== undefined && diag()!.freelistBytes! > 0}>
                <div class="kv-row">
                  <span class="kv-k">Reclaimable (freelist)</span>
                  <span class="kv-v">{formatBytes(diag()!.freelistBytes)}</span>
                </div>
              </Show>
              <Show when={info()!.browser.persisted !== undefined}>
                <div class="kv-row">
                  <span class="kv-k">Eviction-protected</span>
                  <span class="kv-v">
                    {info()!.browser.persisted ? 'yes (navigator.storage.persist granted)' : 'no'}
                    <Show when={info()!.browser.persisted === false}>
                      {' '}
                      <button class="storage-persist-btn" onClick={() => void handlePersist()}>
                        Request persistent storage
                      </button>
                      <Show when={persistResult() !== null}>
                        <span class="muted"> {persistResult() ? 'granted' : 'denied'}</span>
                      </Show>
                    </Show>
                  </span>
                </div>
              </Show>
            </div>
          </div>

          {/* 5. Bucket file cache */}
          <Show when={blobs()}>
            <div class="mcp-section">
              <h3>Bucket file cache</h3>
              <Show when={!blobs()!.persistent}>
                <div class="storage-error-text">
                  Not persisting — running in tab memory only. Cached files will not survive a
                  reload (OPFS unwritable, or the storage quota was exhausted).
                </div>
              </Show>
              <Show when={blobs()!.persistPaused}>
                <div class="storage-error-text">
                  Over budget with nothing evictable: every remaining entry is pinned or on
                  screen. New files are not being cached. Raise <code>blobCache.maxBytes</code>{' '}
                  or unpin something.
                </div>
              </Show>
              <Show when={blobBudgetPct() !== null}>
                <div
                  class="storage-usage-bar"
                  title={`${blobBudgetPct()!.toFixed(1)}% of the blob cache budget`}
                >
                  <div class="storage-usage-fill" style={{ width: `${blobBudgetPct()}%` }} />
                </div>
              </Show>
              <div class="kv">
                <div class="kv-row">
                  <span class="kv-k">Cached files</span>
                  <span class="kv-v">
                    {blobs()!.entries} — {formatBytes(blobs()!.totalBytes)} of{' '}
                    {formatBytes(blobs()!.budgetBytes)} budget
                    {blobBudgetPct() !== null ? ` (${blobBudgetPct()!.toFixed(1)}%)` : ''}
                  </span>
                </div>
                <Show when={blobs()!.pinnedBytes > 0}>
                  <div class="kv-row">
                    <span class="kv-k">Pinned</span>
                    <span class="kv-v">
                      {formatBytes(blobs()!.pinnedBytes)} (never evicted under pressure)
                    </span>
                  </div>
                </Show>
                <div class="kv-row">
                  <span class="kv-k">Hits / misses</span>
                  <span class="kv-v">
                    {blobs()!.hits} / {blobs()!.misses}
                  </span>
                </div>
                <Show when={blobs()!.evictedEntries > 0}>
                  <div class="kv-row">
                    <span class="kv-k">Evicted this session</span>
                    <span class="kv-v">
                      {blobs()!.evictedEntries} files — {formatBytes(blobs()!.evictedBytes)}
                    </span>
                  </div>
                </Show>
                <div class="kv-row">
                  <span class="kv-k">Reconciled at boot</span>
                  <span class="kv-v">
                    {blobs()!.reconciledEntries} files found on disk
                  </span>
                </div>
              </div>
            </div>
          </Show>

          {/* 6. OPFS files */}
          <div class="mcp-section">
            <h3>OPFS files</h3>
            <Show
              when={info()!.opfs.supported}
              fallback={<div class="muted">OPFS is not supported in this browser context.</div>}
            >
              <Show when={realError(info()!.opfs.error)}>
                <div class="storage-error-text">{realError(info()!.opfs.error)}</div>
              </Show>
              <Show
                when={info()!.opfs.entries.length > 0}
                fallback={<div class="muted">OPFS is empty.</div>}
              >
                <div class="storage-file-list">
                  <For each={info()!.opfs.entries}>
                    {(entry) => (
                      <div
                        class="storage-file-row"
                        classList={{
                          dir: entry.kind === 'directory',
                          active: topLevelOf(entry) === activePoolDir(),
                          orphan: isOrphanPool(entry),
                          nested: entry.path.includes('/'),
                        }}
                      >
                        <span class="storage-file-path mono">{entry.path}</span>
                        <span class="storage-file-meta">
                          <Show when={entry.kind === 'directory'}>
                            <Show when={topLevelOf(entry) === activePoolDir() && !entry.path.includes('/')}>
                              <span class="status-pill status-active">
                                <span class="status-dot" />
                                active pool
                              </span>
                            </Show>
                            <Show when={isOrphanPool(entry) && !entry.path.includes('/')}>
                              <span class="status-pill status-initializing">
                                <span class="status-dot" />
                                orphaned pool (other bucket)
                              </span>
                            </Show>
                          </Show>
                          <Show when={entry.kind === 'file'}>
                            {entry.size !== undefined
                              ? formatBytes(entry.size)
                              : lockedFileNote(entry)}
                          </Show>
                        </span>
                      </div>
                    )}
                  </For>
                </div>
                <div class="storage-file-total muted">
                  Total (readable files): {formatBytes(info()!.opfs.totalBytes)}
                  <Show when={info()!.opfs.truncated}> — listing truncated</Show>
                </div>
              </Show>
            </Show>
          </div>

          {/* 6. SQLite stats */}
          <Show when={stats()}>
            <div class="mcp-section">
              <h3>SQLite stats</h3>
              <div class="kv">
                <div class="kv-row">
                  <span class="kv-k">Worker round-trips</span>
                  <span class="kv-v">
                    {stats()!.roundTrips ?? 0}
                    <Show when={stats()!.proxiedOps !== undefined}>
                      <span class="muted">
                        {' '}
                        — {stats()!.proxiedOps} of them through the leader tab's port
                      </span>
                    </Show>
                  </span>
                </div>
                <div class="kv-row">
                  <span class="kv-k">By op</span>
                  <span class="kv-v">
                    {Object.entries(stats()!.byType ?? {})
                      .map(([t, n]) => `${t}: ${n}`)
                      .join(', ') || '—'}
                  </span>
                </div>
                <div class="kv-row">
                  <span class="kv-k">Queue wait</span>
                  <span class="kv-v">{formatMs(stats()!.queueWaitMs)}</span>
                </div>
                <div class="kv-row">
                  <span class="kv-k">Worker time</span>
                  <span class="kv-v">{formatMs(stats()!.workerMs)}</span>
                </div>
                <div class="kv-row">
                  <span class="kv-k">RPC overhead</span>
                  <span class="kv-v">{formatMs(stats()!.rpcOverheadMs)}</span>
                </div>
                <div class="kv-row">
                  <span class="kv-k">Parse time</span>
                  <span class="kv-v">{formatMs(stats()!.parseMs)}</span>
                </div>
                <div class="kv-row">
                  <span class="kv-k">Rows / bytes parsed</span>
                  <span class="kv-v">
                    {stats()!.rowsParsed ?? 0} rows / {formatBytes(stats()!.bytesParsed ?? 0)}
                  </span>
                </div>
                <div class="kv-row">
                  <span class="kv-k">Relation fetches</span>
                  <span class="kv-v">{stats()!.relationFetches ?? 0}</span>
                </div>
                <div class="kv-row">
                  <span class="kv-k">Batch statements</span>
                  <span class="kv-v">
                    {stats()!.batchStatements ?? 0} (max batch {stats()!.maxBatch ?? 0})
                  </span>
                </div>
                <div class="kv-row">
                  <span class="kv-k">In flight</span>
                  <span class="kv-v">
                    {stats()!.inFlight ?? 0} (max {stats()!.maxInFlight ?? 0})
                  </span>
                </div>
              </div>
            </div>
          </Show>

          {/* 7. Per-table rows (opt-in: COUNT(*) per table isn't free) */}
          <div class="mcp-section">
            <h3>Table rows</h3>
            <Show
              when={diag()?.tableCounts}
              fallback={
                <Show
                  when={info()!.engine.kind === 'sqlite'}
                  fallback={<div class="muted">Row counts are only available on the SQLite engine.</div>}
                >
                  <button
                    class="storage-persist-btn"
                    disabled={isFetchingStorage()}
                    onClick={() => void fetchStorageInfo({ tableCounts: true })}
                  >
                    Count rows
                  </button>
                </Show>
              }
            >
              <div class="kv">
                <For each={diag()!.tableCounts}>
                  {(t) => (
                    <div class="kv-row">
                      <span class="kv-k mono">{t.table}</span>
                      <span class="kv-v">{t.rows}</span>
                    </div>
                  )}
                </For>
              </div>
            </Show>
          </div>

          {/* 8. Raw */}
          <details class="storage-raw">
            <summary>Raw snapshot</summary>
            <JsonView value={info()} />
          </details>
        </Show>
      </Show>
    </div>
  );
}
