// Recipe: optimistic-mutation
// Insert a new `{{table}}` row from a UI action. Local cache updates immediately;
// the mutation drains to the remote in the background.

import { createSignal } from 'solid-js';
import { Uuid, useDb } from '@spooky-sync/client-solid';
import { schema } from '../schema.gen';

export function Create{{TablePascal}}Form() {
  const db = useDb<typeof schema>();
  const [submitting, setSubmitting] = createSignal(false);

  const handleSubmit = async (e: SubmitEvent) => {
    e.preventDefault();
    setSubmitting(true);
    try {
      const id = new Uuid().toString();
      await db.create(`{{table}}:${id}`, {
        // TODO: fill in the fields your `{{table}}` schema requires.
      });
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <form onSubmit={handleSubmit}>
      <button type="submit" disabled={submitting()}>
        {submitting() ? 'Creating…' : 'Create {{table}}'}
      </button>
    </form>
  );
}
