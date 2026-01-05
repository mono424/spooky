import { Show, createSignal, createEffect } from "solid-js";

export interface EditingCell {
  recordId: string;
  column: string;
}

interface CellProps {
  value: unknown;
  column: string;
  recordId: string;
  isEditing: boolean;
  onStartEdit: (column: string) => void;
  onUpdate: (value: unknown) => void;
  onCancel: () => void;
  onIdClick?: (id: string) => void;
}

function formatValue(value: unknown): string {
  if (value === undefined || value === null) return "";
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}

function parseValue(value: string): unknown {
  // Try to parse JSON for objects
  if (value.startsWith("{") || value.startsWith("[")) {
    try {
      return JSON.parse(value);
    } catch {
      // Keep as string if parsing fails
      return value;
    }
  } else if (value === "true" || value === "false") {
    return value === "true";
  } else if (!isNaN(Number(value)) && value !== "") {
    return Number(value);
  }
  return value;
}

// Helper to detect if a string is a Record ID (e.g., "table:id")
// Now allows hyphens, which are common in generated IDs
function isRecordId(value: string): boolean {
  return typeof value === 'string' && /^[a-zA-Z0-9_-]+:[a-zA-Z0-9_-]+$/.test(value);
}

export function Cell(props: CellProps) {
  const [editValue, setEditValue] = createSignal("");
  const [inputRef, setInputRef] = createSignal<HTMLInputElement | null>(null);
  const isReadonly = props.column === "id";

  // Initialize edit value and focus input when editing starts
  createEffect(() => {
    if (props.isEditing) {
      setEditValue(formatValue(props.value));
      // Focus input when it becomes available
      setTimeout(() => {
        const input = inputRef();
        if (input) {
          input.focus();
          input.select();
        }
      }, 0);
    }
  });

  const handleClick = (e: MouseEvent) => {
    if (props.isEditing) return;

    // If it's a record ID and we have a click handler, treat as link
    const valStr = formatValue(props.value);
    if (isRecordId(valStr) && props.onIdClick) {
      e.preventDefault();
      e.stopPropagation();
      props.onIdClick(valStr);
      return;
    }

    if (isReadonly) return;
    
    e.stopPropagation();
    props.onStartEdit(props.column);
    setEditValue(formatValue(props.value));
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      e.stopPropagation();
      const parsedValue = parseValue(editValue());
      props.onUpdate(parsedValue);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      props.onCancel();
    }
  };

  const handleBlur = () => {
    // Use setTimeout to allow click events to process first
    setTimeout(() => {
      if (props.isEditing) {
        const parsedValue = parseValue(editValue());
        props.onUpdate(parsedValue);
      }
    }, 200);
  };

  const displayValue = formatValue(props.value);
  const isLink = isRecordId(displayValue);

  return (
    <td
      class="editable-cell"
      classList={{ 
        editing: props.isEditing, 
        readonly: isReadonly,
        link: isLink && !props.isEditing
      }}
      onClick={handleClick}
      title={isLink ? `Go to ${displayValue}` : displayValue}
      style={isLink && !props.isEditing ? { cursor: "pointer" } : {}}
    >
      <Show when={props.isEditing} fallback={
        <div style={{ display: "flex", "align-items": "center", gap: "4px" }}>
          <Show when={isLink}>
            <svg 
              viewBox="0 0 24 24" 
              width="10" 
              height="10" 
              fill="none" 
              stroke="currentColor" 
              stroke-width="2" 
              stroke-linecap="round" 
              stroke-linejoin="round"
              style={{ opacity: 0.7, "min-width": "10px" }}
            >
              <line x1="7" y1="17" x2="17" y2="7"></line>
              <polyline points="7 7 17 7 17 17"></polyline>
            </svg>
          </Show>
          <span class="cell-value" style={{ "white-space": "nowrap", "overflow": "hidden", "text-overflow": "ellipsis" }}>{displayValue}</span>
        </div>
      }>
        <input
          ref={setInputRef}
          type="text"
          class="cell-input"
          value={editValue()}
          onInput={(e) => setEditValue(e.currentTarget.value)}
          onBlur={handleBlur}
          onKeyDown={handleKeyDown}
          onClick={(e) => e.stopPropagation()}
        />
      </Show>
    </td>
  );
}
