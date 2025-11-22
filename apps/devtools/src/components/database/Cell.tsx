import { Show, createSignal, createEffect } from "solid-js";

export interface EditingCell {
  rowIndex: number;
  column: string;
}

interface CellProps {
  value: unknown;
  column: string;
  rowIndex: number;
  isEditing: boolean;
  onStartEdit: (rowIndex: number, column: string) => void;
  onUpdate: (value: unknown) => void;
  onCancel: () => void;
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
    if (isReadonly || props.isEditing) return;
    e.stopPropagation();
    props.onStartEdit(props.rowIndex, props.column);
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

  return (
    <td
      class="editable-cell"
      classList={{ editing: props.isEditing, readonly: isReadonly }}
      onClick={handleClick}
    >
      <Show when={props.isEditing} fallback={displayValue}>
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
