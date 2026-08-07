import { createEffect, createMemo, For, Show } from 'solid-js';
import { useDevTools } from '../../context/DevToolsContext';
import { JsonView } from '../ui/JsonView';
import type { FlagRow } from '../../types/devtools';

/**
 * Two sections, deliberately separate because they have very different blast
 * radii:
 *
 *  - **Local overrides** change what THIS browser resolves. No auth, no
 *    network, works signed out. Always visible.
 *  - **Flags** change the flag for EVERY user. Admin only (`spky admin add`),
 *    enforced by SurrealDB rather than by hiding the UI.
 */
export function FlagsTab() {
  const {
    flagsSnapshot,
    flagsError,
    isMutatingFlag,
    fetchFlags,
    isSp00kyAvailable,
    setFlagEnabled,
    setFlagUserVariant,
    setFlagOverride,
    clearFlagOverrides,
  } = useDevTools();

  // The tab body is unmounted while inactive (App.tsx wraps it in <Show>), so
  // this runs on every open — the snapshot is a point-in-time remote read.
  //
  // Gated on availability because the panel can mount before the page's client
  // is up, and an early call can only time out (30s) rather than fail fast.
  //
  // `attempted` rather than inferring intent from `flagsSnapshot()`: fetchFlags
  // CATCHES its errors, so on failure the snapshot stays null while
  // isFetchingFlags flips true -> false — which re-runs this effect, which
  // refetches, forever, one 30s remote read after another. A plain `let` dies
  // with the component, so re-opening the tab retries exactly once.
  let attempted = false;
  createEffect(() => {
    if (!isSp00kyAvailable()) return;
    if (attempted || flagsSnapshot()) return;
    attempted = true;
    void fetchFlags();
  });

  const snap = () => flagsSnapshot();
  const overrides = () => snap()?.overrides ?? {};

  /**
   * Rows for the override table: every flag this browser knows about, whether
   * it came from an assignment, an override, or (for admins) a definition.
   * A key with only an override still needs a row, otherwise there'd be no way
   * to clear it.
   */
  const overrideRows = createMemo(() => {
    const s = snap();
    if (!s) return [];
    const keys = new Set<string>();
    for (const a of s.assignments) keys.add(a.key);
    for (const k of Object.keys(s.overrides)) keys.add(k);
    for (const f of s.flags) keys.add(f.key);

    return [...keys].sort().map((key) => {
      const assigned = s.assignments.find((a) => a.key === key)?.variant;
      const definition = s.flags.find((f) => f.key === key);
      // Variants come from the definition when we can see it; otherwise infer
      // what we can, so a non-admin still gets a usable picker.
      const variants = definition?.variants?.length
        ? definition.variants
        : [...new Set(['off', 'on', assigned, s.overrides[key]?.variant].filter(Boolean))];
      return { key, assigned, override: s.overrides[key]?.variant, variants: variants as string[] };
    });
  });

  const statusPill = () => {
    const s = snap();
    if (!s) return { label: 'Loading', cls: 'status-initializing' };
    if (!s.userId) return { label: 'Signed out', cls: 'status-destroyed' };
    if (s.isAdmin) return { label: 'Admin', cls: 'status-active' };
    return { label: 'Read-only', cls: 'status-updating' };
  };

  return (
    <div class="mcp-container">
      <div class="mcp-header">
        <h2>Feature Flags</h2>
        <div class="mcp-header-controls">
          {/* Refresh lives in the top toolbar now — it re-reads flags when this
              tab is active. */}
          <div class={`status-pill ${statusPill().cls}`}>
            <span class="status-dot" />
            {statusPill().label}
          </div>
        </div>
      </div>

      <Show when={flagsError()}>
        <div class="storage-health-banner error">{flagsError()}</div>
      </Show>

      {/* ---- Local overrides ------------------------------------------- */}
      <div class="mcp-section">
        <h3>Local overrides</h3>
        <p class="muted">
          Forces a variant in this browser only. Nothing is sent to the server, so clearing an
          override restores whatever the server says. Survives reloads.
        </p>

        <Show
          when={overrideRows().length > 0}
          fallback={
            <div class="empty-state">
              No flags seen yet. Assignments arrive once you sign in and the app calls{' '}
              <code>client.feature(...)</code>.
            </div>
          }
        >
          <table class="data-table flags-override-table">
            <thead>
              <tr>
                <th>Flag</th>
                <th>Assigned</th>
                <th>Override</th>
                <th />
              </tr>
            </thead>
            <tbody>
              <For each={overrideRows()}>
                {(row) => (
                  <tr classList={{ 'flags-overridden': !!row.override }}>
                    <td class="mono">{row.key}</td>
                    <td class="mono muted">{row.assigned ?? '—'}</td>
                    <td>
                      <select
                        class="flags-variant-select"
                        value={row.override ?? ''}
                        onChange={(e) =>
                          void setFlagOverride(row.key, e.currentTarget.value || null)
                        }
                      >
                        <option value="">(none)</option>
                        <For each={row.variants}>
                          {(variant) => <option value={variant}>{variant}</option>}
                        </For>
                      </select>
                    </td>
                    <td>
                      <Show when={row.override}>
                        <button
                          class="icon-btn"
                          title="Clear override"
                          onClick={() => void setFlagOverride(row.key, null)}
                        >
                          ✕
                        </button>
                      </Show>
                    </td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>

          <Show when={Object.keys(overrides()).length > 0}>
            <button class="btn" onClick={() => void clearFlagOverrides()}>
              Clear all overrides
            </button>
          </Show>
        </Show>
      </div>

      {/* ---- Remote flags (admin only) --------------------------------- */}
      <div class="mcp-section">
        <h3>Flags (all users)</h3>

        <Show when={snap()?.isAdmin} fallback={<AdminFallback />}>
          <p class="muted">
            Changes here apply to <strong>every user</strong> and take effect live. Creating,
            deleting and percentage rollouts stay with <code>spky flag</code>.
          </p>

          <Show
            when={(snap()?.flags.length ?? 0) > 0}
            fallback={
              <div class="empty-state">
                No flags defined. Create one with <code>spky flag create &lt;key&gt;</code>.
              </div>
            }
          >
            <For each={snap()!.flags}>
              {(flag) => (
                <FlagCard
                  flag={flag}
                  busy={isMutatingFlag() === flag.key}
                  anyBusy={isMutatingFlag() !== null}
                  onToggle={(enabled) => void setFlagEnabled(flag.key, enabled)}
                  onVariant={(variant, remove) =>
                    void setFlagUserVariant(flag.key, variant, remove)
                  }
                />
              )}
            </For>
          </Show>
        </Show>
      </div>
    </div>
  );
}

/**
 * Why the admin section is empty. These are four genuinely different problems
 * with four different fixes, so they must not collapse into one empty state.
 */
function AdminFallback() {
  const { flagsSnapshot, flagsError } = useDevTools();
  const s = () => flagsSnapshot();

  return (
    <Show when={s()} fallback={<div class="empty-state">Loading…</div>}>
      <Show
        when={s()!.userId}
        fallback={
          <div class="empty-state">
            Sign in to manage flags. Local overrides above work while signed out.
          </div>
        }
      >
        <Show
          when={!flagsError()}
          fallback={
            <div class="empty-state">
              Couldn't reach the flag tables. If this deployment predates the Flags tab, run{' '}
              <code>spky migrate</code> (or redeploy) to apply the internal schema.
            </div>
          }
        >
          <div class="empty-state">
            You're not an admin, so flag definitions are hidden from this client. Grant access with{' '}
            <code class="mono">spky admin add {shortUser(s()!.userId!)}</code>.
          </div>
        </Show>
      </Show>
    </Show>
  );
}

function shortUser(id: string): string {
  return id.startsWith('user:') ? id : `user:${id}`;
}

function FlagCard(props: {
  flag: FlagRow;
  busy: boolean;
  anyBusy: boolean;
  onToggle: (enabled: boolean) => void;
  onVariant: (variant: string, remove: boolean) => void;
}) {
  const allowlisted = () => props.flag.selfAllowlistedVariant;

  return (
    <div class="flags-card" classList={{ 'flags-busy': props.busy }}>
      <div class="flags-card-head">
        <span class="mono flags-key">{props.flag.key}</span>
        <label class="mcp-toggle" title="Enable/disable for all users">
          <input
            type="checkbox"
            checked={props.flag.enabled}
            // Disabled while ANY flag is mutating: materialize writes
            // `_00_user_feature` rows under a UNIQUE (user, key) index, and
            // overlapping runs can collide.
            disabled={props.anyBusy}
            onChange={(e) => props.onToggle(e.currentTarget.checked)}
          />
          <span class="mcp-toggle-slider" />
        </label>
      </div>

      <Show when={props.flag.description}>
        <p class="muted">{props.flag.description}</p>
      </Show>

      <div class="kv">
        <div class="kv-row">
          <span class="kv-k">Default</span>
          <span class="kv-v mono">{props.flag.default_variant}</span>
        </div>
        <div class="kv-row">
          <span class="kv-k">You</span>
          <span class="kv-v mono">
            {allowlisted() ? `allowlisted → ${allowlisted()}` : 'not allowlisted'}
          </span>
        </div>
        <Show when={props.flag.updated_at}>
          <div class="kv-row">
            <span class="kv-k">Updated</span>
            <span class="kv-v mono muted">{props.flag.updated_at}</span>
          </div>
        </Show>
      </div>

      <div class="flags-variant-group">
        <span class="kv-k">Set for me:</span>
        <For each={props.flag.variants}>
          {(variant) => (
            <button
              class="btn"
              classList={{ 'flags-variant-active': allowlisted() === variant }}
              disabled={props.anyBusy}
              onClick={() => props.onVariant(variant, false)}
            >
              {variant}
            </button>
          )}
        </For>
        <Show when={allowlisted()}>
          <button
            class="btn delete-btn"
            disabled={props.anyBusy}
            onClick={() => props.onVariant(allowlisted()!, true)}
          >
            Remove me
          </button>
        </Show>
      </div>

      <Show when={props.flag.rules.length > 0 || props.flag.payloads}>
        <details class="detail-section">
          <summary>Rules &amp; payloads</summary>
          <JsonView
            value={{ rules: props.flag.rules, payloads: props.flag.payloads }}
            class="row-pane-json"
          />
        </details>
      </Show>
    </div>
  );
}
