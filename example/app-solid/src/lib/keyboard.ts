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

type KeyCombo = string; // e.g. "ctrl+k", "Shift+?", "g h" (sequence)
type KeyHandler = (e: KeyboardEvent) => void;

interface ShortcutOptions {
  preventDefault?: boolean;
}

export function useKeyboard(
  shortcuts: Record<KeyCombo, KeyHandler>,
  options: ShortcutOptions = { preventDefault: true }
) {
  // Track pending prefix for key sequences (e.g. "g" in "g h")
  let pendingPrefix = '';
  let pendingTimer: ReturnType<typeof setTimeout> | null = null;

  // Collect sequence prefixes from shortcuts like "g h"
  const sequencePrefixes = new Set<string>();
  for (const key of Object.keys(shortcuts)) {
    if (key.includes(' ')) {
      sequencePrefixes.add(key.split(' ')[0]);
    }
  }

  const clearPending = () => {
    pendingPrefix = '';
    if (pendingTimer) {
      clearTimeout(pendingTimer);
      pendingTimer = null;
    }
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    // Always allow Escape to trigger even in inputs (for closing modals/blurring)
    if (e.key === 'Escape') {
      clearPending();
      if (shortcuts['Escape']) {
        shortcuts['Escape'](e);
        return;
      }
    }

    // Ignore other shortcuts if typing
    if (isInputActive()) return;

    // Check if this key completes a pending sequence
    if (pendingPrefix) {
      const sequence = `${pendingPrefix} ${e.key}`;
      clearPending();
      if (shortcuts[sequence]) {
        if (options.preventDefault) e.preventDefault();
        shortcuts[sequence](e);
        return;
      }
    }

    // Check if this key starts a sequence
    if (sequencePrefixes.has(e.key)) {
      pendingPrefix = e.key;
      pendingTimer = setTimeout(clearPending, 1000);
      // Don't return yet — also check for a direct match below
      // so single-key shortcuts still work even if they're also prefixes
      if (!shortcuts[e.key]) return;
    }

    // Build combo string
    const parts = [];
    if (e.ctrlKey) parts.push('Ctrl');
    if (e.metaKey) parts.push('Meta');
    if (e.altKey) parts.push('Alt');
    if (e.shiftKey) parts.push('Shift');

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
    clearPending();
    window.removeEventListener('keydown', handleKeyDown);
  });
}
