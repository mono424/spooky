import { createSignal, onMount, onCleanup, Show } from 'solid-js';
import { useDb } from '@spooky-sync/client-solid';

export function PendingMutationsIndicator() {
  const db = useDb();
  const [count, setCount] = createSignal(db.pendingMutationCount);
  const [visible, setVisible] = createSignal(count() > 0);

  let hideTimeout: ReturnType<typeof setTimeout> | undefined;

  onMount(() => {
    const unsub = db.subscribeToPendingMutations((newCount) => {
      setCount(newCount);
      if (newCount > 0) {
        clearTimeout(hideTimeout);
        setVisible(true);
      } else {
        hideTimeout = setTimeout(() => setVisible(false), 400);
      }
    });

    onCleanup(() => {
      unsub();
      clearTimeout(hideTimeout);
    });
  });

  return (
    <Show when={visible()}>
      <div
        class={`pending-mutations-pill ${count() === 0 ? 'pending-mutations-exit' : ''}`}
      >
        <span class="pending-mutations-dot" />
        <span class="text-xs font-medium tracking-wide">
          {count()} pending
        </span>
      </div>
    </Show>
  );
}
