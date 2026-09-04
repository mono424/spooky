import { Show, createEffect, createSignal } from 'solid-js';
import { currentMode, login, setEndpoint } from '../api/client';
import type { LoginResponse } from '../api/types';
import { Logo } from '../components/Chrome';

/**
 * Sign-in.
 *
 * The endpoint field appears only when this bundle is NOT being served by a
 * scheduler. Embedded, the endpoint is where the page came from — asking for it
 * again would be asking the operator to retype something we already know.
 */
export function Login(props: { onSignedIn: (session: LoginResponse) => void }) {
  const [endpoint, setEndpointValue] = createSignal(currentMode().baseUrl);
  const [username, setUsername] = createSignal('');
  const [password, setPassword] = createSignal('');
  const [breakglass, setBreakglass] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [config, setConfig] = createSignal(currentMode().config);

  const embedded = () => currentMode().embedded;

  // A standalone dashboard cannot know whether break-glass is offered until it
  // has reached a scheduler, so re-read the config whenever it changes.
  createEffect(() => {
    setConfig(currentMode().config);
  });

  const connect = async () => {
    setError(null);
    setBusy(true);
    try {
      setConfig(await setEndpoint(endpoint()));
    } catch {
      setError('Could not reach a scheduler at that address.');
      setConfig(null);
    } finally {
      setBusy(false);
    }
  };

  const submit = async (e: Event) => {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      // A standalone form where the operator typed an endpoint but never
      // pressed Connect should still just work.
      if (!embedded() && !config()) {
        setConfig(await setEndpoint(endpoint()));
      }
      const session = await login(
        password(),
        breakglass() ? undefined : username(),
      );
      props.onSignedIn(session);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Sign-in failed');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="login-wrap">
      <form class="login-card" onSubmit={submit}>
        <div class="login-title">
          <Logo />
          <span class="brand-tag">admin</span>
        </div>
        <div class="login-sub">
          <Show
            when={config()}
            fallback="Connect to a scheduler to sign in."
          >
            {(c) => (
              <>
                {c().scheduler_id} · v{c().version}
              </>
            )}
          </Show>
        </div>

        <Show when={!embedded()}>
          <div class="field">
            <label for="endpoint">Scheduler endpoint</label>
            <input
              id="endpoint"
              placeholder="http://10.0.0.5:9668"
              value={endpoint()}
              onInput={(e) => setEndpointValue(e.currentTarget.value)}
              onBlur={connect}
              autocomplete="url"
            />
          </div>
        </Show>

        <Show when={!breakglass()}>
          <div class="field">
            <label for="username">Username</label>
            <input
              id="username"
              value={username()}
              onInput={(e) => setUsername(e.currentTarget.value)}
              autocomplete="username"
              autofocus
            />
          </div>
        </Show>

        <div class="field">
          <label for="password">
            {breakglass() ? 'Admin password' : 'Password'}
          </label>
          <input
            id="password"
            type="password"
            value={password()}
            onInput={(e) => setPassword(e.currentTarget.value)}
            autocomplete="current-password"
          />
        </div>

        <Show when={error()}>
          <div class="login-error">{error()}</div>
        </Show>

        <button
          class="btn btn-primary"
          type="submit"
          disabled={busy()}
          style={{ width: '100%' }}
        >
          {busy() ? 'Signing in…' : 'Sign in'}
        </button>

        {/* Offered only when the scheduler says a break-glass password is
            configured, so the option never appears where it cannot work. */}
        <Show when={config()?.breakglass_available}>
          <div style={{ 'margin-top': '14px', 'text-align': 'center' }}>
            <button
              type="button"
              class="link-btn"
              onClick={() => {
                setBreakglass(!breakglass());
                setError(null);
              }}
            >
              {breakglass()
                ? 'Sign in with an admin account'
                : 'Use the break-glass admin password'}
            </button>
          </div>
        </Show>

        <Show when={!breakglass()}>
          <div
            class="faint"
            style={{ 'margin-top': '16px', 'font-size': '11.5px', 'text-align': 'center' }}
          >
            Accounts are granted access with <span class="mono">spky admin add</span>.
          </div>
        </Show>
      </form>
    </div>
  );
}
