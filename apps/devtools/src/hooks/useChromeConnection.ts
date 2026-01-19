import { createSignal, onMount, onCleanup } from 'solid-js';
import type { ChromeMessage } from '../types/devtools';

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
      console.warn('[DevTools] Cannot send message: not connected', message);
    }
  };

  /**
   * Request Spooky state from the inspected page
   */
  const requestState = (): void => {
    sendMessage({ type: 'GET_SPOOKY_STATE' });
  };

  onMount(() => {
    let retryTimeout: number | undefined;

    const connect = () => {
      try {
        // Connect to the background script
        const newPort = chrome.runtime.connect({ name: 'spooky-devtools' });

        // Listen for messages from background script
        const messageListener = (message: ChromeMessage) => {
          console.log('[DevTools] Received message from background:', message);
          options?.onMessage?.(message);
        };
        newPort.onMessage.addListener(messageListener);

        // Handle disconnection
        const disconnectListener = () => {
          console.log('[DevTools] Disconnected from background script');
          setIsConnected(false);
          setPort(null);
          options?.onDisconnect?.();

          // Attempt to reconnect after 2 seconds
          retryTimeout = setTimeout(connect, 2000);
        };
        newPort.onDisconnect.addListener(disconnectListener);

        // Set up connection state
        setPort(newPort);
        setIsConnected(true);

        // Initialize connection with tab ID
        newPort.postMessage({
          name: 'init',
          tabId: chrome.devtools.inspectedWindow.tabId,
        });

        console.log('[DevTools] Connected to background script');
        options?.onConnect?.();

        return { newPort, messageListener, disconnectListener };
      } catch (e) {
        console.error('[DevTools] Connection failed:', e);
        // Retry on immediate failure too
        retryTimeout = setTimeout(connect, 2000);
        return null;
      }
    };

    let activeConnection = connect();

    // Cleanup on unmount
    onCleanup(() => {
      console.log('[DevTools] Cleaning up connection');
      clearTimeout(retryTimeout);

      if (activeConnection) {
        const { newPort, messageListener, disconnectListener } = activeConnection;
        try {
          newPort.onMessage.removeListener(messageListener);
          newPort.onDisconnect.removeListener(disconnectListener);
          newPort.disconnect();
        } catch (e) {
          /* ignore */
        }
      }

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
