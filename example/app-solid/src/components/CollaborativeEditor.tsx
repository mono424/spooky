import { createEffect, onCleanup, onMount } from 'solid-js';
import { Editor, Extension } from '@tiptap/core';
import StarterKit from '@tiptap/starter-kit';
import Placeholder from '@tiptap/extension-placeholder';
import { LoroSyncPlugin, LoroUndoPlugin, undo, redo } from 'loro-prosemirror';
import type { CrdtField } from '@spooky-sync/core';
import { keymap } from '@tiptap/pm/keymap';

interface CollaborativeEditorProps {
  field: CrdtField;
  content?: string;
  placeholder?: string;
  class?: string;
  editable?: boolean;
  onUpdate?: (text: string) => void;
  singleLine?: boolean;
}

export function CollaborativeEditor(props: CollaborativeEditorProps) {
  let containerRef: HTMLDivElement | undefined;
  let editor: Editor | undefined;
  let userHasInteracted = false;
  let lastEmittedText: string | undefined;
  let suppressOnUpdate = true;

  onMount(() => {
    if (!containerRef) return;

    const crdtField = props.field;
    const doc = crdtField.getDoc();
    const hasCrdtState = crdtField.hasContent();
    const fallback = props.content;

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

    if (!hasCrdtState && fallback && editor.getText().length === 0) {
      editor.commands.setContent(textToHtml(fallback));
    }

    lastEmittedText = editor.getText();
    setTimeout(() => { suppressOnUpdate = false; }, 100);
  });

  createEffect(() => {
    if (editor) editor.setEditable(props.editable !== false);
  });

  onCleanup(() => { editor?.destroy(); });

  return <div ref={containerRef} class={props.class ?? ''} />;
}

function textToHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
    .split('\n').map((l) => `<p>${l || '<br>'}</p>`).join('');
}

