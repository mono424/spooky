// Recipe: crdt-text-field
// Wire a CRDT text column (`{{table}}.{{field}}` annotated `-- @crdt text` in the schema)
// to a textarea. Concurrent edits from multiple clients merge via Loro.

import { useDb, useQuery, useCrdtField } from '@spooky-sync/client-solid';
import { schema } from '../schema.gen';

export function {{TablePascal}}{{FieldPascal}}Editor(props: { id: string }) {
  const db = useDb<typeof schema>();

  // Pull the surrounding row so we know the field is loaded.
  const rowResult = useQuery(() =>
    db.query('{{table}}').where({ id: props.id } as any).build()
  );
  const row = () => rowResult.data()?.[0];

  // Bind the CRDT field. All four args take accessor functions for SolidJS tracking.
  const field = useCrdtField(
    '{{table}}',
    () => row()?.id,
    '{{field}}',
    () => row()?.{{field}}
  );

  const handleInput = async (e: InputEvent) => {
    const next = (e.currentTarget as HTMLTextAreaElement).value;
    const r = row();
    if (!r?.id) return;
    // `debounced` coalesces rapid keystrokes into a single mutation.
    await db.update('{{table}}', r.id, { {{field}}: next }, { debounced: true });
  };

  return (
    <textarea value={field.value() ?? ''} onInput={handleInput} />
  );
}
