import { Show, type JSX } from 'solid-js';

/** Small shared pieces of the panel chrome. */

export function PageHead(props: {
  crumb?: string;
  title: string;
  subtitle?: JSX.Element;
  actions?: JSX.Element;
}) {
  return (
    <div class="page-head">
      <div style={{ 'min-width': '0' }}>
        <Show when={props.crumb}>
          <div class="crumb">{props.crumb}</div>
        </Show>
        <h1>{props.title}</h1>
        <Show when={props.subtitle}>
          <div class="id" style={{ 'margin-top': '5px' }}>
            {props.subtitle}
          </div>
        </Show>
      </div>
      <Show when={props.actions}>
        <div class="row">{props.actions}</div>
      </Show>
    </div>
  );
}

export function StatusDot(props: { tone: string; pulse?: boolean }) {
  return (
    <span
      class="dot"
      classList={{ [props.tone]: true, pulse: !!props.pulse }}
    />
  );
}

export function Pill(props: {
  tone?: string;
  dot?: boolean;
  pulse?: boolean;
  children: JSX.Element;
}) {
  return (
    <span class="pill" classList={{ [props.tone ?? '']: !!props.tone }}>
      <Show when={props.dot}>
        <StatusDot tone={props.tone ?? 'idle'} pulse={props.pulse} />
      </Show>
      {props.children}
    </span>
  );
}

/** One readout in a metric rail. */
export function Cell(props: {
  label: string;
  value: JSX.Element;
  unit?: string;
  foot?: JSX.Element;
  tone?: string;
  pulse?: boolean;
}) {
  return (
    <div class="rail-cell">
      <div class="rail-label">
        <Show when={props.tone}>
          <StatusDot tone={props.tone!} pulse={props.pulse} />
        </Show>
        {props.label}
      </div>
      <div class="rail-value">
        {props.value}
        <Show when={props.unit}>
          <span class="rail-unit">{props.unit}</span>
        </Show>
      </div>
      <Show when={props.foot}>
        <div class="rail-foot">{props.foot}</div>
      </Show>
    </div>
  );
}

export function Rail(props: { children: JSX.Element }) {
  return <div class="rail">{props.children}</div>;
}

export function Panel(props: {
  title?: string;
  sub?: JSX.Element;
  actions?: JSX.Element;
  flush?: boolean;
  children: JSX.Element;
}) {
  return (
    <div class="panel">
      <Show when={props.title || props.actions}>
        <div class="panel-head">
          <div>
            <Show when={props.title}>
              <h2>{props.title}</h2>
            </Show>
            <Show when={props.sub}>
              <div class="panel-sub">{props.sub}</div>
            </Show>
          </div>
          <Show when={props.actions}>
            <div class="row">{props.actions}</div>
          </Show>
        </div>
      </Show>
      {props.flush ? props.children : <div class="panel-body">{props.children}</div>}
    </div>
  );
}

export function Empty(props: { children: JSX.Element }) {
  return <div class="empty">{props.children}</div>;
}

/** An identifier with a copy affordance — ids here are long and get pasted. */
export function CopyId(props: { value: string }) {
  return (
    <span class="row" style={{ gap: '0' }}>
      <span class="id">{props.value}</span>
      <button
        class="copy"
        title="Copy"
        onClick={(e) => {
          e.stopPropagation();
          void navigator.clipboard?.writeText(props.value);
          const el = e.currentTarget;
          const original = el.textContent;
          el.textContent = 'copied';
          setTimeout(() => (el.textContent = original), 1200);
        }}
      >
        copy
      </button>
    </span>
  );
}

export function KeyValue(props: { rows: [string, JSX.Element][] }) {
  return (
    <dl class="kv">
      {props.rows.map(([k, v]) => (
        <>
          <dt>{k}</dt>
          <dd>{v}</dd>
        </>
      ))}
    </dl>
  );
}
