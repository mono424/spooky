import { For, Show, createEffect, createSignal, onCleanup, type JSX } from 'solid-js';
import { A, useLocation } from '@solidjs/router';
import type { LoginResponse, Overview } from '../api/types';

interface NavLink {
  href: string;
  label: string;
  count?: string;
}

/**
 * Sidebar + content frame.
 *
 * Nav counts come from the overview poll the app already runs, so the rail is
 * live without a request of its own.
 *
 * Below 860px the rail becomes an off-canvas drawer behind a top bar (see
 * theme.css): 208px of a 375px viewport is more than half the screen spent on
 * navigation. The drawer is CSS-driven; this component owns only the open
 * state and the things JS has to do — close on navigation, close on Escape,
 * and stop the page behind it scrolling.
 */
export function Shell(props: {
  session: LoginResponse;
  overview: Overview | undefined;
  onSignOut: () => void;
  children: JSX.Element;
}) {
  const location = useLocation();
  const [open, setOpen] = createSignal(false);

  const links = (): NavLink[] => [
    { href: '/', label: 'Overview' },
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
    { href: '/workflows', label: 'Workflows' },
    { href: '/schedules', label: 'Schedules' },
    { href: '/backups', label: 'Backups' },
    { href: '/logs', label: 'Logs' },
    { href: '/access', label: 'Access' },
  ];

  // Navigating closes the drawer — otherwise tapping a link leaves it sitting
  // open over the page it just moved to.
  createEffect(() => {
    location.pathname;
    setOpen(false);
  });

  // An open drawer must not let the page behind it scroll under the scrim.
  createEffect(() => {
    document.body.style.overflow = open() ? 'hidden' : '';
  });
  onCleanup(() => {
    document.body.style.overflow = '';
  });

  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') setOpen(false);
  };
  window.addEventListener('keydown', onKey);
  onCleanup(() => window.removeEventListener('keydown', onKey));

  return (
    <div class="shell">
      {/* Mobile only; `display:none` above the breakpoint. */}
      <header class="mobile-bar">
        <button
          class="burger"
          classList={{ open: open() }}
          aria-label={open() ? 'Close navigation' : 'Open navigation'}
          aria-expanded={open()}
          onClick={() => setOpen(!open())}
        >
          <span />
          <span />
          <span />
        </button>
        <span class="brand-mark">Sp00ky</span>
      </header>

      <Show when={open()}>
        <div class="scrim show" onClick={() => setOpen(false)} />
      </Show>

      <nav class="sidebar" classList={{ open: open() }}>
        <div class="brand">
          <span class="brand-mark">Sp00ky</span>
          <span class="brand-version">
            {props.overview?.scheduler?.version ?? ''}
          </span>
        </div>

        <div class="nav">
          <For each={links()}>
            {(link) => (
              // `activeClass` + `end` rather than comparing pathnames by hand:
              // the router applies the base ("/admin") to every href, so a
              // hand-rolled `location.pathname === href` never matches and the
              // rail silently highlights nothing. `end` keeps "/" from
              // matching every route as a prefix.
              <A
                href={link.href}
                class="nav-link"
                activeClass="active"
                end={link.href === '/'}
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
