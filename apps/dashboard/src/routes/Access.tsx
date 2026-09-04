import { For, Show, createResource, createSignal } from 'solid-js';
import { api, currentMode } from '../api/client';
import { Empty, PageHead, Panel, Pill } from '../components/Chrome';
import { toast } from '../components/Actions';
import { formatStamp } from '../lib/format';
import type { McpTool, MeResponse, SessionScope, TokenResponse } from '../api/types';

/**
 * MCP access: mint a token an AI agent can use against this scheduler's own
 * MCP endpoint, and show what that agent will be able to do.
 *
 * The token is shown exactly once. It is a signed session with a long life,
 * not a row anywhere, so the server cannot show it again; the page says so
 * rather than offering a "reveal" that would have nothing to reveal.
 */

const LIFETIMES = [7, 30, 90, 365] as const;

interface JsonRpcResult<T> {
  result?: T;
  error?: { code: number; message: string };
}

async function listTools(): Promise<McpTool[]> {
  const res = await api.post<JsonRpcResult<{ tools: McpTool[] }>>('/mcp', {
    jsonrpc: '2.0',
    id: 1,
    method: 'tools/list',
  });
  if (res.error) throw new Error(res.error.message);
  return res.result?.tools ?? [];
}

/** Where an MCP client must point: the page origin, or the standalone endpoint. */
function endpointBase(): string {
  const mode = currentMode();
  return mode.embedded ? window.location.origin : mode.baseUrl;
}

function serverName(): string {
  const slug = currentMode().config?.project_slug;
  return slug ? `spky-admin-${slug}` : 'spky-admin';
}

/** Group a tool by its name prefix, so the list reads by area. */
function groupOf(name: string): string {
  if (/^(workflow_|schedule_|schedules_|job_)/.test(name)) return 'Workflows';
  // Before the `ssp` rule: `ssp` alone would otherwise swallow nothing here,
  // but the presence tools have no shared prefix of their own, so they are
  // matched by name.
  if (/^(presence|views_|view_)/.test(name)) return 'Views';
  if (/^(ssp|scheduler_|cloud_)/.test(name)) return 'Restart';
  if (/^backup/.test(name)) return 'Backups';
  return 'Cluster';
}

const GROUP_ORDER = ['Cluster', 'Views', 'Workflows', 'Restart', 'Backups'];

function toolTone(t: McpTool): { label: string; tone: string } {
  if (t.annotations?.destructiveHint) return { label: 'destructive', tone: 'bad' };
  if (t.annotations?.readOnlyHint) return { label: 'read', tone: 'idle' };
  return { label: 'write', tone: 'warn' };
}

function CopyButton(props: { value: string; label?: string }) {
  const [done, setDone] = createSignal(false);
  return (
    <button
      class="btn btn-sm"
      onClick={() => {
        void navigator.clipboard?.writeText(props.value);
        setDone(true);
        setTimeout(() => setDone(false), 1400);
      }}
    >
      {done() ? 'copied' : (props.label ?? 'copy')}
    </button>
  );
}

function Snippet(props: { title: string; file?: string; code: string }) {
  return (
    <div class="snippet">
      <div class="snippet-head">
        <span class="tag">
          {props.title}
          <Show when={props.file}>
            {' '}
            <span class="ghost">{props.file}</span>
          </Show>
        </span>
        <CopyButton value={props.code} />
      </div>
      <pre>{props.code}</pre>
    </div>
  );
}

export function Access() {
  const [me] = createResource(() => api.getResult<MeResponse>('/me'));
  const [tools, { refetch: refetchTools }] = createResource(async () => {
    try {
      return { ok: true as const, tools: await listTools() };
    } catch (err) {
      return {
        ok: false as const,
        message: err instanceof Error ? err.message : 'tools/list failed',
      };
    }
  });

  const [label, setLabel] = createSignal('');
  const [scope, setScope] = createSignal<SessionScope>('read');
  const [ttl, setTtl] = createSignal<number>(90);
  const [busy, setBusy] = createSignal(false);
  const [minted, setMinted] = createSignal<TokenResponse | null>(null);
  const [revokeValue, setRevokeValue] = createSignal('');

  const config = () => currentMode().config;
  const persistent = () => config()?.sessions_persistent !== false;
  const canMint = () => {
    const m = me();
    if (!m || !m.ok) return true;
    return m.value.mode !== 'mcp' && (m.value.scope ?? 'full') === 'full';
  };

  const mcpUrl = (t: TokenResponse) => `${endpointBase()}${t.endpoint}`;

  const claudeCmd = (t: TokenResponse) =>
    `claude mcp add --transport http ${serverName()} ${mcpUrl(t)} \\\n  --header "Authorization: Bearer ${t.token}" --scope user`;

  const cursorJson = (t: TokenResponse) =>
    JSON.stringify(
      {
        mcpServers: {
          [serverName()]: {
            url: mcpUrl(t),
            headers: { Authorization: `Bearer ${t.token}` },
          },
        },
      },
      null,
      2,
    );

  const vscodeJson = (t: TokenResponse) =>
    JSON.stringify(
      {
        servers: {
          [serverName()]: {
            type: 'http',
            url: mcpUrl(t),
            headers: { Authorization: `Bearer ${t.token}` },
          },
        },
      },
      null,
      2,
    );

  const mint = async (e: Event) => {
    e.preventDefault();
    if (busy()) return;
    setBusy(true);
    try {
      const t = await api.post<TokenResponse>('/tokens', {
        label: label().trim() || 'mcp',
        scope: scope(),
        ttl_days: ttl(),
      });
      setMinted(t);
      toast('ok', 'Token created', 'Copy it now; it is not shown again.');
    } catch (err) {
      toast('bad', 'Could not create token', err instanceof Error ? err.message : undefined);
    } finally {
      setBusy(false);
    }
  };

  const revoke = async (e: Event) => {
    e.preventDefault();
    const value = revokeValue().trim();
    if (!value || busy()) return;
    setBusy(true);
    try {
      await api.delete('/tokens', { token: value });
      setRevokeValue('');
      toast('ok', 'Token revoked');
    } catch (err) {
      toast('bad', 'Could not revoke', err instanceof Error ? err.message : undefined);
    } finally {
      setBusy(false);
    }
  };

  const grouped = () => {
    const t = tools();
    if (!t || !t.ok) return [];
    const map = new Map<string, McpTool[]>();
    for (const tool of t.tools) {
      const g = groupOf(tool.name);
      map.set(g, [...(map.get(g) ?? []), tool]);
    }
    return GROUP_ORDER.filter((g) => map.has(g)).map((g) => ({
      group: g,
      tools: map.get(g)!,
    }));
  };

  return (
    <>
      <PageHead
        crumb="Dashboard"
        title="Access"
        subtitle={
          <Show when={me()?.ok && me()}>
            {(m) => (
              <>
                signed in as {(m() as { ok: true; value: MeResponse }).value.label}
                {' · '}
                {(m() as { ok: true; value: MeResponse }).value.mode}
                {' · '}
                {(m() as { ok: true; value: MeResponse }).value.scope ?? 'full'}
              </>
            )}
          </Show>
        }
      />

      <div class="page-body">
        <div class="stack">
          <Show when={!persistent()}>
            <div class="banner">
              <span class="dot warn" />
              No <span class="dim">SPKY_AUTH_SECRET</span> is set, so tokens live in this
              scheduler process only and die with it. Set the secret to mint tokens that
              survive a restart.
            </div>
          </Show>

          <div class="grid grid-2">
            <Panel
              title="MCP access"
              sub="A token an AI agent presents to this scheduler's MCP endpoint"
            >
              <Show
                when={canMint()}
                fallback={
                  <Empty>
                    This session cannot mint tokens: only a full, interactive sign-in can.
                  </Empty>
                }
              >
                <form class="stack" style={{ gap: '12px' }} onSubmit={mint}>
                  <div>
                    <label class="tag" style={{ display: 'block', 'margin-bottom': '6px' }}>
                      Label
                    </label>
                    <input
                      placeholder="e.g. claude-code on my laptop"
                      value={label()}
                      onInput={(e) => setLabel(e.currentTarget.value)}
                      style={{ width: '100%' }}
                    />
                  </div>

                  <div>
                    <div class="tag" style={{ 'margin-bottom': '6px' }}>
                      Scope
                    </div>
                    <div class="choice-list">
                      <label class="choice" classList={{ on: scope() === 'read' }}>
                        <input
                          type="radio"
                          name="scope"
                          checked={scope() === 'read'}
                          onChange={() => setScope('read')}
                        />
                        <div>
                          <div class="choice-title">Read-only</div>
                          <div class="choice-sub">
                            Overview, backends, logs, runs, schedules, backups catalog.
                            Every write is refused with a 403.
                          </div>
                        </div>
                      </label>
                      <label class="choice" classList={{ on: scope() === 'full' }}>
                        <input
                          type="radio"
                          name="scope"
                          checked={scope() === 'full'}
                          onChange={() => setScope('full')}
                        />
                        <div>
                          <div class="choice-title">Full</div>
                          <div class="choice-sub">
                            Everything the dashboard can do: restarts, cancels, retries,
                            backups and restores. Same blast radius as your own session.
                          </div>
                        </div>
                      </label>
                    </div>
                  </div>

                  <div>
                    <label class="tag" style={{ display: 'block', 'margin-bottom': '6px' }}>
                      Lifetime
                    </label>
                    <select
                      value={String(ttl())}
                      onChange={(e) => setTtl(Number(e.currentTarget.value))}
                    >
                      <For each={LIFETIMES}>
                        {(d) => (
                          <option value={String(d)}>
                            {d} days
                          </option>
                        )}
                      </For>
                    </select>
                  </div>

                  <div class="row">
                    <button class="btn btn-primary" type="submit" disabled={busy()}>
                      Create token
                    </button>
                  </div>
                </form>
              </Show>

              <Show when={minted()}>
                {(t) => (
                  <div class="stack" style={{ gap: '10px', 'margin-top': '18px' }}>
                    <div class="banner" style={{ 'margin-bottom': '0' }}>
                      <span class="dot warn" />
                      Shown once. Copy it now; the scheduler keeps no record of it.
                    </div>
                    <div class="token-box">
                      <span class="token-value">{t().token}</span>
                      <CopyButton value={t().token} />
                    </div>
                    <div class="ghost" style={{ 'font-size': '11px' }}>
                      {t().label} · {t().scope} · expires {formatStamp(t().expires_at)}
                    </div>
                    <Snippet title="Claude Code" code={claudeCmd(t())} />
                    <Snippet title="Cursor" file="~/.cursor/mcp.json" code={cursorJson(t())} />
                    <Snippet title="VS Code" file=".vscode/mcp.json" code={vscodeJson(t())} />
                  </div>
                )}
              </Show>
            </Panel>

            <div class="stack">
              <Panel title="Endpoint" sub="Streamable HTTP, JSON-RPC over POST">
                <div class="token-box" style={{ 'border-color': 'var(--rule)' }}>
                  <span class="token-value">{endpointBase()}/admin/api/mcp</span>
                  <CopyButton value={`${endpointBase()}/admin/api/mcp`} />
                </div>
                <div class="ghost prose" style={{ 'margin-top': '10px', 'font-size': '11.5px' }}>
                  Any admin session token works as the bearer too; a minted token is only
                  a longer-lived one with an explicit scope.
                </div>
              </Panel>

              <Panel title="Revoke a token" sub="Takes effect immediately for this process">
                <form class="row" onSubmit={revoke}>
                  <input
                    placeholder="paste the token"
                    value={revokeValue()}
                    onInput={(e) => setRevokeValue(e.currentTarget.value)}
                    style={{ flex: '1' }}
                  />
                  <button class="btn" type="submit" disabled={busy() || !revokeValue().trim()}>
                    Revoke
                  </button>
                </form>
                <Show when={persistent()}>
                  <div class="ghost" style={{ 'margin-top': '8px', 'font-size': '11px' }}>
                    Revocations are kept in memory; a scheduler restart forgets them until the
                    token's own expiry.
                  </div>
                </Show>
              </Panel>
            </div>
          </div>

          <Panel
            title="Tools"
            sub="What an agent can call, as this scheduler lists them"
            actions={
              <button class="btn btn-sm" onClick={() => void refetchTools()}>
                refresh
              </button>
            }
          >
            <Show when={tools()} fallback={<Empty>Loading…</Empty>}>
              {(t) => (
                <Show
                  when={t().ok}
                  fallback={<Empty>{(t() as { ok: false; message: string }).message}</Empty>}
                >
                  <For each={grouped()}>
                    {(g) => (
                      <div class="tool-group">
                        <div class="tag" style={{ 'margin-bottom': '4px' }}>
                          {g.group}
                        </div>
                        <For each={g.tools}>
                          {(tool) => (
                            <div class="tool-row">
                              <span class="tool-name">{tool.name}</span>
                              <Pill tone={toolTone(tool).tone}>{toolTone(tool).label}</Pill>
                              <span class="tool-desc">{tool.description}</span>
                            </div>
                          )}
                        </For>
                      </div>
                    )}
                  </For>
                </Show>
              )}
            </Show>
          </Panel>
        </div>
      </div>
    </>
  );
}
