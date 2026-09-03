import { For, Show, type JSX } from 'solid-js';
import { A, useLocation } from '@solidjs/router';
import type { LoginResponse, Overview } from '../api/types';

/**
 * Sidebar + content frame.
 *
 * Nav counts come from the overview poll the app already runs, so the rail is
 * live without a request of its own.
 */
export function Shell(props: {
  session: LoginResponse;
  overview: Overview | undefined;
  onSignOut: () => void;
  children: JSX.Element;
}) {
  const location = useLocation();

  const links = () => [
    { href: '/', label: 'Overview', count: undefined as string | undefined },
    {
      href: '/ssps',
      label: 'SSPs',
      count: props.overview
        ? `${props.overview.totals.ssps_ready}/${props.overview.totals.ssps}`
        : undefined,
    },
    {
      href: '/backends',
      label: 'Backends',
      count: props.overview
        ? `${props.overview.totals.backends_healthy}/${props.overview.totals.backends}`
        : undefined,
    },
    { href: '/workflows', label: 'Workflows', count: undefined },
    { href: '/schedules', label: 'Schedules', count: undefined },
    { href: '/logs', label: 'Logs', count: undefined },
  ];

  // `/` must match exactly, or it lights up on every route.
  const isActive = (href: string) =>
    href === '/' ? location.pathname === '/' : location.pathname.startsWith(href);

  return (
    <div class="shell">
      <nav class="sidebar">
        <div class="brand">
          <span class="brand-mark">Sp00ky</span>
          <span class="brand-version">
            {props.overview?.scheduler?.version ?? ''}
          </span>
        </div>

        <div class="nav">
          <For each={links()}>
            {(link) => (
              <A
                href={link.href}
                class="nav-link"
                classList={{ active: isActive(link.href) }}
              >
                <span>{link.label}</span>
                <Show when={link.count}>
                  <span class="nav-count">{link.count}</span>
                </Show>
              </A>
            )}
          </For>
        </div>

        <div class="sidebar-foot">
          <span class="truncate" title={props.session.subject}>
            {props.session.label}
          </span>
          <button class="link-btn" onClick={props.onSignOut}>
            Sign out
          </button>
        </div>
      </nav>

      <main class="main">
        {props.children}
        <Show when={props.session.mode === 'breakglass'}>
          <div class="page-body">
            <div class="banner" style={{ 'margin-top': '16px' }}>
              <span class="dot warn" />
              Signed in with the break-glass password — the{' '}
              <span class="dim">_00_admin</span> roster was bypassed.
            </div>
          </div>
        </Show>
      </main>
    </div>
  );
}
