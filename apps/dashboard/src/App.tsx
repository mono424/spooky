import { Show, createEffect, createSignal, onCleanup, onMount } from 'solid-js';
import { Route, Router } from '@solidjs/router';
import {
  api,
  currentMode,
  getToken,
  logout,
  onUnauthorized,
  resolveMode,
} from './api/client';
import type { LoginResponse, Overview as OverviewData } from './api/types';
import { Shell } from './components/Shell';
import { Login } from './routes/Login';
import { Overview } from './routes/Overview';
import { Ssps } from './routes/Ssps';
import { Backends, BackendDetailView } from './routes/Backends';
import { Workflows } from './routes/Workflows';
import { WorkflowDetail } from './routes/WorkflowDetail';
import { ScheduleDetail, Schedules } from './routes/Schedules';
import { Logs } from './routes/Logs';

/** How often the overview is refreshed. Fast enough to feel live, slow enough
 *  that a wall display is not a load source. */
const OVERVIEW_INTERVAL = 3000;

export function App() {
  const [ready, setReady] = createSignal(false);
  const [session, setSession] = createSignal<LoginResponse | null>(null);
  const [overview, setOverview] = createSignal<OverviewData | undefined>();
  const [overviewError, setOverviewError] = createSignal<string | undefined>();

  onMount(async () => {
    await resolveMode();

    // A token in storage may be from a previous scheduler process — sessions do
    // not survive a restart — so it has to be checked rather than trusted.
    const token = getToken();
    if (token) {
      try {
        const me = await api.get<{
          subject: string;
          label: string;
          mode: LoginResponse['mode'];
        }>('/me');
        setSession({
          token,
          subject: me.subject,
          label: me.label,
          mode: me.mode,
          expires_in_secs: 0,
        });
      } catch {
        /* stale token; the login form is the right place to land */
      }
    }
    setReady(true);
  });

  // The server rejecting our token is the authority on being signed out.
  onCleanup(onUnauthorized(() => setSession(null)));

  // One poll for the whole app: the shell's nav counts, the overview, the SSP
  // list and the log source picker all read from it.
  const poll = async () => {
    if (!session()) return;
    try {
      setOverview(await api.get<OverviewData>('/overview'));
      setOverviewError(undefined);
    } catch (err) {
      setOverviewError(
        err instanceof Error ? err.message : 'Could not reach the scheduler',
      );
    }
  };

  const timer = setInterval(poll, OVERVIEW_INTERVAL);
  onCleanup(() => clearInterval(timer));

  // Poll once as soon as a session exists, rather than waiting out the first
  // interval. Without this, a reload with a stored token leaves the nav counts
  // and the whole overview blank for OVERVIEW_INTERVAL.
  createEffect(() => {
    if (session()) void poll();
  });

  const signIn = (s: LoginResponse) => {
    setSession(s);
  };

  const signOut = async () => {
    await logout();
    setSession(null);
    setOverview(undefined);
  };

  return (
    <Show when={ready()} fallback={<div class="login-wrap" />}>
      <Show when={session()} fallback={<Login onSignedIn={signIn} />}>
        {(s) => (
          <Router
            // Embedded, the app is served under /admin; standalone (a dev
            // server or a static host) it sits at the root.
            base={currentMode().embedded ? '/admin' : ''}
            root={(props) => (
              <Shell
                session={s()}
                overview={overview()}
                onSignOut={signOut}
              >
                {props.children}
              </Shell>
            )}
          >
            <Route
              path="/"
              component={() => (
                <Overview data={overview()} error={overviewError()} />
              )}
            />
            <Route path="/ssps" component={() => <Ssps data={overview()} />} />
            <Route path="/backends" component={Backends} />
            <Route path="/backends/:name" component={BackendDetailView} />
            <Route path="/workflows" component={Workflows} />
            <Route path="/workflows/:id" component={WorkflowDetail} />
            <Route path="/schedules" component={Schedules} />
            <Route path="/schedules/:name" component={ScheduleDetail} />
            <Route path="/logs" component={() => <Logs overview={overview()} />} />
            <Route path="*" component={() => <Overview data={overview()} />} />
          </Router>
        )}
      </Show>
    </Show>
  );
}
