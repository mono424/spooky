import { Show } from 'solid-js';
import { useFeatureFlag } from '@spooky-sync/client-solid';

/**
 * Demo of the built-in feature flag primitive.
 *
 * Toggle the flag from a terminal and watch this component flip without a
 * page refresh:
 *
 *   spky flag create demo
 *   spky flag enable demo --for-user <your-username>     # variant `on`
 *   spky flag disable demo                                # variant `off`
 *
 * The hook subscribes to `_00_user_feature` for the signed-in user via the
 * same SSP + sync pipeline as `useQuery`, so updates arrive over the
 * existing websocket. Permissions are enforced by SurrealDB: a malicious
 * client cannot self-enable the flag (the table only permits writes from
 * the root token used by the CLI / scheduler).
 */
export function FeatureFlagDemo() {
  const flag = useFeatureFlag('demo');

  return (
    <div data-testid="feature-flag-demo" style={{ padding: '12px', border: '1px solid #ccc', 'border-radius': '6px' }}>
      <div>
        <strong>demo</strong> variant:{' '}
        <code data-testid="feature-flag-demo-variant">{flag.variant() ?? '(unset)'}</code>
      </div>
      <Show
        when={flag.enabled()}
        fallback={<p data-testid="feature-flag-demo-state">Default experience.</p>}
      >
        <p data-testid="feature-flag-demo-state">New experience enabled for this user.</p>
      </Show>
      <Show when={flag.payload()}>
        <pre data-testid="feature-flag-demo-payload">
          {JSON.stringify(flag.payload(), null, 2)}
        </pre>
      </Show>
    </div>
  );
}
