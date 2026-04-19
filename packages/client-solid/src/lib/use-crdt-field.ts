import { createEffect, createSignal, onCleanup, useContext, type Accessor } from 'solid-js';
import { Sp00kyContext } from './context';
import type { CrdtField } from '@spooky-sync/core';

export function useCrdtField(
  table: string,
  recordId: () => string | undefined,
  field: string,
  fallbackText?: () => string | undefined,
): Accessor<CrdtField | null> {
  const db = useContext(Sp00kyContext);
  if (!db) {
    throw new Error('useCrdtField must be used within a <Sp00kyProvider>');
  }

  const [crdtField, setCrdtField] = createSignal<CrdtField | null>(null);
  let currentId: string | undefined;
  let initialized = false;

  createEffect(() => {
    const id = recordId();

    // Skip if the ID hasn't changed (but allow the first non-undefined value through)
    if (initialized && id === currentId) return;

    // Close previous field
    if (currentId && crdtField()) {
      db.getSp00ky().closeCrdtField(table, currentId, field);
      setCrdtField(null);
    }

    currentId = id;
    initialized = true;

    if (!id) return;

    const sp00ky = db.getSp00ky();
    const text = fallbackText?.();
    sp00ky.openCrdtField(table, id, field, text).then((cf) => {
      if (currentId === id) {
        setCrdtField(cf);
      }
    });
  });

  onCleanup(() => {
    if (currentId && crdtField()) {
      db.getSp00ky().closeCrdtField(table, currentId, field);
      setCrdtField(null);
    }
  });

  return crdtField;
}
