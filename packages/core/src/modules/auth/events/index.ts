import { createEventSystem, EventDefinition, EventSystem } from '../../../events/index.js';

export const AuthEventTypes = {
  AuthStateChanged: 'AUTH_STATE_CHANGED',
} as const;

export type AuthEventTypeMap = {
  [AuthEventTypes.AuthStateChanged]: EventDefinition<
    typeof AuthEventTypes.AuthStateChanged,
    string | null
  >;
};

export type AuthEventSystem = EventSystem<AuthEventTypeMap>;

export function createAuthEventSystem(): AuthEventSystem {
  return createEventSystem([AuthEventTypes.AuthStateChanged]);
}
