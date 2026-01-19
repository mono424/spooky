import { createSignal, createEffect, Show, onCleanup } from "solid-js";

export interface ToastProps {
  message: string;
  type?: "error" | "success" | "info";
  duration?: number;
  onDismiss?: () => void;
}

export function Toast(props: ToastProps) {
  const [visible, setVisible] = createSignal(true);

  createEffect(() => {
    if (props.duration !== 0) {
      const timer = setTimeout(() => {
        setVisible(false);
        props.onDismiss?.();
      }, props.duration || 3000);
      onCleanup(() => clearTimeout(timer));
    }
  });

  return (
    <Show when={visible()}>
      <div
        style={{
          position: "fixed",
          bottom: "20px",
          right: "20px",
          padding: "12px 16px",
          background: props.type === "error" ? "var(--sys-color-error, #d32f2f)" : "var(--sys-color-surface-container-highest, #333)",
          color: "white",
          "border-radius": "4px",
          "box-shadow": "0 2px 8px rgba(0,0,0,0.2)",
          "z-index": 9999,
          display: "flex",
          "align-items": "center",
          gap: "8px",
          "font-family": "var(--sys-typescale-body-font)",
          "font-size": "13px",
          "animation": "slideIn 0.3s ease-out"
        }}
      >
        <span>{props.message}</span>
        <button
          onClick={() => {
            setVisible(false);
            props.onDismiss?.();
          }}
          style={{
            background: "transparent",
            border: "none",
            color: "white",
            cursor: "pointer",
            "font-size": "16px",
            padding: "0 4px"
          }}
        >
          ×
        </button>
      </div>
      <style>
        {`
          @keyframes slideIn {
            from { transform: translateY(20px); opacity: 0; }
            to { transform: translateY(0); opacity: 1; }
          }
        `}
      </style>
    </Show>
  );
}
