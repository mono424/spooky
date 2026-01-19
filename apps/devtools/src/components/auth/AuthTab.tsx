import { Show } from 'solid-js';
import { useDevTools } from '../../context/DevToolsContext';
import { formatTime, formatRelativeTime } from '../../utils/formatters';

export function AuthTab() {
  const { state } = useDevTools();

  return (
    <div class="auth-container">
      <div class="auth-header">
        <h2>Authentication</h2>
      </div>
      <div class="auth-info">
        <div
          class="auth-status"
          classList={{
            authenticated: state.auth.isAuthenticated,
            'not-authenticated': !state.auth.isAuthenticated,
          }}
        >
          <div>
            <strong>Status:</strong>{' '}
            {state.auth.isAuthenticated ? 'Authenticated' : 'Not authenticated'}
          </div>

          <Show when={state.auth.user}>
            <div style="margin-top: 12px;">
              <strong>User:</strong>
              <div style="margin-left: 12px; margin-top: 4px;">
                <Show when={state.auth.user?.email}>
                  <div>
                    <strong>Email:</strong> {state.auth.user!.email}
                  </div>
                </Show>
                <Show when={state.auth.user?.roles && state.auth.user.roles.length > 0}>
                  <div style="margin-top: 4px;">
                    <strong>Roles:</strong> {state.auth.user!.roles!.join(', ')}
                  </div>
                </Show>
              </div>
            </div>
          </Show>

          <div style="margin-top: 12px;">
            <strong>Last Check:</strong> {formatTime(state.auth.lastAuthCheck)} (
            {formatRelativeTime(state.auth.lastAuthCheck)})
          </div>
        </div>
      </div>
    </div>
  );
}
