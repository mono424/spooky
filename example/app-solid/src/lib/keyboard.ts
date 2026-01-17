import { createSignal, onCleanup, onMount, createRoot } from 'solid-js';

// Global store for help modal visibility
const [isShortcutsHelpOpen, setIsShortcutsHelpOpen] = createSignal(false);

export const useShortcutsHelp = () => {
  return {
    isOpen: isShortcutsHelpOpen,
    open: () => setIsShortcutsHelpOpen(true),
    close: () => setIsShortcutsHelpOpen(false),
    toggle: () => setIsShortcutsHelpOpen((prev) => !prev),
  };
};

// Helper to check if user is typing in an input
export const isInputActive = () => {
  const activeElement = document.activeElement;
  return (
    activeElement instanceof HTMLInputElement ||
    activeElement instanceof HTMLTextAreaElement ||
    (activeElement instanceof HTMLElement && activeElement.isContentEditable)
  );
};

type KeyCombo = string; // e.g. "ctrl+k", "Shift+?"
type KeyHandler = (e: KeyboardEvent) => void;

interface ShortcutOptions {
  preventDefault?: boolean;
}

export function useKeyboard(
  shortcuts: Record<KeyCombo, KeyHandler>,
  options: ShortcutOptions = { preventDefault: true }
) {
  const handleKeyDown = (e: KeyboardEvent) => {
    // Always allow Escape to trigger even in inputs (for closing modals/blurring)
    if (e.key === 'Escape') {
      if (shortcuts['Escape']) {
        shortcuts['Escape'](e);
        return;
      }
    }

    // Ignore other shortcuts if typing
    if (isInputActive()) return;

    // Build combo string
    const parts = [];
    if (e.ctrlKey) parts.push('Ctrl');
    if (e.metaKey) parts.push('Meta');
    if (e.altKey) parts.push('Alt');
    if (e.shiftKey) parts.push('Shift');

    // Handle special keys or regular keys
    // We capitalize first letter for consistency with our map keys if needed,
    // but generally e.key is enough.
    // For '?' e.key is '?' (with shift held), but we might map it as '?' or 'Shift+/'

    // Simple strategy: Try exact key match first (e.g. "?"), then modifiers (e.g. "Ctrl+k")
    if (shortcuts[e.key]) {
      if (options.preventDefault) e.preventDefault();
      shortcuts[e.key](e);
      return;
    }

    // If modifiers are present, try to construct combo
    if (
      parts.length > 0 &&
      e.key !== 'Control' &&
      e.key !== 'Shift' &&
      e.key !== 'Alt' &&
      e.key !== 'Meta'
    ) {
      const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
      const combo = [...parts, key].join('+'); // e.g. Ctrl+k

      // Also try lowercase key for single letters
      if (shortcuts[combo]) {
        if (options.preventDefault) e.preventDefault();
        shortcuts[combo](e);
      }
    }
  };

  onMount(() => {
    window.addEventListener('keydown', handleKeyDown);
  });

  onCleanup(() => {
    window.removeEventListener('keydown', handleKeyDown);
  });
}
