import type {
  BackendDevToolsState,
  DevToolsState,
  SpookyEvent,
  ActiveQuery,
  AuthState,
} from "../types/devtools";

/**
 * Transforms backend DevTools state to frontend state structure
 */
export function adaptBackendState(
  backendState: BackendDevToolsState
): DevToolsState {
  // Transform events
  const events: SpookyEvent[] = backendState.eventsHistory.map((event) => ({
    type: event.eventType,
    timestamp: event.timestamp,
    data: event.payload,
  }));

  // Transform activeQueries from Record to Array
  const activeQueries: ActiveQuery[] = Object.values(
    backendState.activeQueries
  );

  // Transform auth state
  const auth: AuthState = {
    isAuthenticated: backendState.auth.authenticated,
    user: backendState.auth.userId
      ? {
          email: backendState.auth.userId,
          roles: [],
        }
      : null,
    lastAuthCheck: backendState.auth.timestamp || Date.now(),
  };

  return {
    events,
    activeQueries,
    auth,
    database: backendState.database,
  };
}
