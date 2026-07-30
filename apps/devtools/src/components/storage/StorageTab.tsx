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
    (): { status: string; fallback: boolean; error?: string } =>
      state.database.storage ?? storageInfo()?.health ?? { status: 'unknown', fallback: false }
  );

  const info = storageInfo;
  const diag = () => info()?.engineDiagnostics;
  const stats = () => info()?.sqliteStats as Record<string, any> | undefined;

  const usagePct = createMemo(() => {
    const b = info()?.browser;
    if (!b?.quota || b.usage === undefined) return null;
    return Math.min(100, (b.usage / b.quota) * 100);
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
          <div class="storage-health-hint">
            The whole dataset sits in RAM and every local write is lost on reload. The usual cause
            is another tab of this app holding the OPFS pool lock — close other tabs and reload.
          </div>
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
          </span>
          <span class="muted">
            {health().status === 'persistent'
              ? 'Local store is OPFS-backed and survives reloads.'
              : health().status === 'memory'
                ? 'In-memory store as configured (store: memory) — nothing persists by design.'
                : 'This engine does not report storage health.'}
          </span>
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

          {/* 3. Space */}
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

          {/* 4. OPFS files */}
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
                              : 'size unavailable — file locked'}
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

          {/* 5. SQLite stats */}
          <Show when={stats()}>
            <div class="mcp-section">
              <h3>SQLite stats</h3>
              <div class="kv">
                <div class="kv-row">
                  <span class="kv-k">Worker round-trips</span>
                  <span class="kv-v">{stats()!.roundTrips ?? 0}</span>
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

          {/* 6. Per-table rows (opt-in: COUNT(*) per table isn't free) */}
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

          {/* 7. Raw */}
          <details class="storage-raw">
            <summary>Raw snapshot</summary>
            <JsonView value={info()} />
          </details>
        </Show>
      </Show>
    </div>
  );
}
