import { createSignal } from 'solid-js';

// Re-export TanStack hotkeys primitives
export { createHotkey, createHotkeySequence } from '@tanstack/solid-hotkeys';

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
