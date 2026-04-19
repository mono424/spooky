import { createEffect, onCleanup, onMount } from 'solid-js';
import { Editor, Extension } from '@tiptap/core';
import StarterKit from '@tiptap/starter-kit';
import Placeholder from '@tiptap/extension-placeholder';
import {
  LoroSyncPlugin,
  LoroUndoPlugin,
  LoroEphemeralCursorPlugin,
  CursorEphemeralStore,
  undo,
  redo,
} from 'loro-prosemirror';
import { cursorColorFromName, textToHtml, type CrdtField } from '@spooky-sync/core';
import { keymap } from '@tiptap/pm/keymap';

interface CollaborativeEditorProps {
  field: CrdtField;
  content?: string;
  placeholder?: string;
  class?: string;
  editable?: boolean;
  onUpdate?: (text: string) => void;
  singleLine?: boolean;
  username?: string;
}

export function CollaborativeEditor(props: CollaborativeEditorProps) {
  let containerRef: HTMLDivElement | undefined;
  let editor: Editor | undefined;
  let userHasInteracted = false;
  let lastEmittedText: string | undefined;
  let suppressOnUpdate = true;
  let presence: CursorEphemeralStore | undefined;
  let cursorPushTimer: ReturnType<typeof setTimeout> | undefined;
  let cursorUnsub: (() => void) | undefined;

  onMount(() => {
    if (!containerRef) return;

    const crdtField = props.field;
    const doc = crdtField.getDoc();
    const hasCrdtState = crdtField.hasContent();
    const fallback = props.content;

    // Create cursor presence store
    const cursorColor = cursorColorFromName(props.username ?? 'Anonymous');
    presence = new CursorEphemeralStore(doc.peerIdStr);
    presence.setLocal({
      user: { name: props.username ?? 'Anonymous', color: cursorColor },
    });

    const presenceRef = presence;

    editor = new Editor({
      element: containerRef,
      extensions: [
        StarterKit.configure({
          history: false,
          ...(props.singleLine ? {
            heading: false, bulletList: false, orderedList: false,
            blockquote: false, codeBlock: false, horizontalRule: false, hardBreak: false,
          } : {}),
        }),
        Placeholder.configure({ placeholder: props.placeholder ?? 'Start typing...' }),
        Extension.create({
          name: 'loroCollaboration',
          addProseMirrorPlugins() {
            return [
              LoroSyncPlugin({ doc: doc as any }),
              LoroUndoPlugin({ doc }),
              keymap({ 'Mod-z': undo, 'Mod-y': redo, 'Mod-Shift-z': redo }),
            ];
          },
        }),
      ],
      editable: props.editable !== false,
      editorProps: {
        attributes: { class: 'outline-none' },
        ...(props.singleLine ? {
          handleKeyDown: (_view: any, event: KeyboardEvent) => {
            if (event.key === 'Enter') { event.preventDefault(); return true; }
            return false;
          },
        } : {}),
      },
      onFocus: () => { userHasInteracted = true; },
      onUpdate: ({ editor: ed }) => {
        if (suppressOnUpdate || !userHasInteracted) return;
        const text = ed.getText();
        if (text === lastEmittedText) return;
        lastEmittedText = text;
        props.onUpdate?.(text);
      },
    });

    // Seed content if LoroDoc was empty
    if (!hasCrdtState && fallback && editor.getText().length === 0) {
      editor.commands.setContent(textToHtml(fallback));
    }

    lastEmittedText = editor.getText();

    // Add cursor plugin after content is settled
    const editorRef = editor;
    setTimeout(() => {
      suppressOnUpdate = false;

      try {
        const cursorPlugin = LoroEphemeralCursorPlugin(presenceRef, {
          user: {
            name: props.username ?? 'Anonymous',
            color: cursorColor,
          },
        });
        const state = editorRef.view.state.reconfigure({
          plugins: [...editorRef.view.state.plugins, cursorPlugin],
        });
        editorRef.view.updateState(state);
      } catch (e) {
        console.warn('[CollaborativeEditor] Failed to add cursor plugin:', e);
      }

      // Sync cursor state: push local cursor changes to _00_crdt as "_cursor" field
      cursorUnsub = presenceRef.subscribeBy((by) => {
        if (by !== 'local') return;
        if (cursorPushTimer) clearTimeout(cursorPushTimer);
        cursorPushTimer = setTimeout(() => {
          crdtField.pushCursorState(presenceRef.encodeAll());
        }, 100);
      });

      // Import remote cursor state when it arrives
      crdtField.onCursorUpdate = (data: Uint8Array) => {
        try { presenceRef.apply(data); } catch (e) {
          console.warn('[CollaborativeEditor] Failed to apply remote cursor:', e);
        }
      };
    }, 200);
  });

  createEffect(() => {
    if (editor) editor.setEditable(props.editable !== false);
  });

  onCleanup(() => {
    if (cursorUnsub) cursorUnsub();
    if (cursorPushTimer) clearTimeout(cursorPushTimer);
    editor?.destroy();
  });

  return <div ref={containerRef} class={props.class ?? ''} />;
}

