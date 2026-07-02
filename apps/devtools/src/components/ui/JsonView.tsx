import { For } from 'solid-js';

type Token = { text: string; cls?: string };

// Strings (with optional trailing `:` marking an object key), booleans, null,
// numbers. Everything between matches (punctuation, whitespace) passes through
// unstyled.
const TOKEN_RE = /("(?:\\.|[^"\\])*")(\s*:)?|\b(?:true|false|null)\b|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/g;

function tokenize(json: string): Token[] {
  const tokens: Token[] = [];
  let last = 0;
  for (const m of json.matchAll(TOKEN_RE)) {
    const idx = m.index ?? 0;
    if (idx > last) tokens.push({ text: json.slice(last, idx) });
    if (m[1] !== undefined) {
      if (m[2] !== undefined) {
        tokens.push({ text: m[1], cls: 'json-key' });
        tokens.push({ text: m[2] });
      } else {
        tokens.push({ text: m[1], cls: 'json-string' });
      }
    } else if (m[0] === 'true' || m[0] === 'false') {
      tokens.push({ text: m[0], cls: 'json-boolean' });
    } else if (m[0] === 'null') {
      tokens.push({ text: m[0], cls: 'json-null' });
    } else {
      tokens.push({ text: m[0], cls: 'json-number' });
    }
    last = idx + m[0].length;
  }
  if (last < json.length) tokens.push({ text: json.slice(last) });
  return tokens;
}

/**
 * Chrome-devtools-style syntax-highlighted JSON block. Pass `class` for
 * container sizing (e.g. `row-pane-json`, `code-block`).
 */
export function JsonView(props: { value: unknown; class?: string }) {
  const tokens = () => {
    const json = JSON.stringify(props.value, null, 2);
    return tokenize(json === undefined ? 'undefined' : json);
  };
  return (
    <pre class={`json-view ${props.class ?? ''}`}>
      <For each={tokens()}>{(t) => (t.cls ? <span class={t.cls}>{t.text}</span> : t.text)}</For>
    </pre>
  );
}
