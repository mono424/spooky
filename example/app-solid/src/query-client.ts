import { QueryClient } from "@tanstack/solid-query";

// Create a QueryClient instance
export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 0, // Always consider data stale to allow live updates
      refetchOnWindowFocus: false, // We handle updates via subscriptions
      refetchOnReconnect: false, // We handle updates via subscriptions
    },
  },
});
