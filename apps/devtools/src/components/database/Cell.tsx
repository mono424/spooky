import { Show } from 'solid-js';

interface CellProps {
  value: unknown;
  onIdClick?: (id: string) => void;
}

function formatValue(value: unknown): string {
  if (value === undefined || value === null) return '';
  if (typeof value === 'object') return JSON.stringify(value);
  return String(value);
}

// Detect a Record ID (e.g. "table:id"). Hyphens are allowed since generated
// IDs commonly contain them.
function isRecordId(value: string): boolean {
  return typeof value === 'string' && /^[a-zA-Z0-9_-]+:[a-zA-Z0-9_-]+$/.test(value);
}

/**
 * Read-only data cell. Values are never editable; the only interaction is
 * clicking a Record ID to jump to its table (handled by `onIdClick`).
 */
export function Cell(props: CellProps) {
  const displayValue = () => formatValue(props.value);
  const isLink = () => isRecordId(displayValue()) && !!props.onIdClick;

  const handleClick = (e: MouseEvent) => {
    if (!isLink()) return;
    e.preventDefault();
    e.stopPropagation();
    props.onIdClick?.(displayValue());
  };

  return (
    <td
      class="data-cell"
      classList={{ link: isLink() }}
      onClick={handleClick}
      title={isLink() ? `Go to ${displayValue()}` : displayValue()}
    >
      <div style={{ display: 'flex', 'align-items': 'center', gap: '4px' }}>
        <Show when={isLink()}>
          <svg
            viewBox="0 0 24 24"
            width="10"
            height="10"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            style={{ opacity: 0.7, 'min-width': '10px' }}
          >
            <line x1="7" y1="17" x2="17" y2="7"></line>
            <polyline points="7 7 17 7 17 17"></polyline>
          </svg>
        </Show>
        <span
          class="cell-value"
          style={{ 'white-space': 'nowrap', overflow: 'hidden', 'text-overflow': 'ellipsis' }}
        >
          {displayValue()}
        </span>
      </div>
    </td>
  );
}
