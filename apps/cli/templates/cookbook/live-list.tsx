// Recipe: live-list
// Reactive, sorted list of `{{table}}` rows. Re-runs automatically as records change.

import { For } from 'solid-js';
import { useQuery, useDb } from '@spooky-sync/client-solid';
import { schema } from '../schema.gen';

export function {{TablePascal}}List() {
  const db = useDb<typeof schema>();

  const result = useQuery(() =>
    db.query('{{table}}').orderBy('id', 'asc').limit(50).build()
  );

  return (
    <ul>
      <For each={result.data() ?? []}>
        {(row) => <li>{row.id}</li>}
      </For>
    </ul>
  );
}
