import { createSignal, onMount, onCleanup } from "solid-js";
import type { ChromeMessage } from "../types/devtools";

export interface UseChromeConnectionOptions {
  onMessage?: (message: ChromeMessage) => void;
  onConnect?: () => void;
  onDisconnect?: () => void;
}

/**
 * Custom hook to manage Chrome runtime connection with background script
 * Handles port connection, message passing, and cleanup
 */
export function useChromeConnection(options?: UseChromeConnectionOptions) {
  const [isConnected, setIsConnected] = createSignal(false);
  const [port, setPort] = createSignal<chrome.runtime.Port | null>(null);

  /**
   * Send a message through the port
   */
  const sendMessage = (message: ChromeMessage): void => {
    const currentPort = port();
    if (currentPort && isConnected()) {
      currentPort.postMessage(message);
    } else {
      console.warn("[DevTools] Cannot send message: not connected", message);
    }
  };

  /**
   * Request Spooky state from the inspected page
   */
  const requestState = (): void => {
    sendMessage({ type: "GET_SPOOKY_STATE" });
  };

  onMount(() => {
    // Connect to the background script
    const newPort = chrome.runtime.connect({ name: "spooky-devtools" });
    setPort(newPort);
    setIsConnected(true);

    // Initialize connection with tab ID
    newPort.postMessage({
      name: "init",
      tabId: chrome.devtools.inspectedWindow.tabId,
    });

    console.log("[DevTools] Connected to background script");
    options?.onConnect?.();

    // Listen for messages from background script
    const messageListener = (message: ChromeMessage) => {
      console.log("[DevTools] Received message from background:", message);
      options?.onMessage?.(message);
    };

    newPort.onMessage.addListener(messageListener);

    // Handle disconnection
    const disconnectListener = () => {
      console.log("[DevTools] Disconnected from background script");
      setIsConnected(false);
      setPort(null);
      options?.onDisconnect?.();
    };

    newPort.onDisconnect.addListener(disconnectListener);

    // Cleanup on unmount
    onCleanup(() => {
      console.log("[DevTools] Cleaning up connection");
      newPort.onMessage.removeListener(messageListener);
      newPort.onDisconnect.removeListener(disconnectListener);
      newPort.disconnect();
      setIsConnected(false);
      setPort(null);
    });
  });

  return {
    isConnected,
    sendMessage,
    requestState,
  };
}
