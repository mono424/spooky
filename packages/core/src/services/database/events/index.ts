import { createEventSystem, EventDefinition, EventSystem } from '../../../events/index.js';

export const DatabaseEventTypes = {
  LocalQuery: 'DATABASE_LOCAL_QUERY',
  RemoteQuery: 'DATABASE_REMOTE_QUERY',
} as const;

export interface DatabaseQueryEventPayload {
  query: string;
  vars?: Record<string, unknown>;
  duration: number;
  success: boolean;
  error?: string;
  timestamp: number;
}

export type DatabaseEventTypeMap = {
  [DatabaseEventTypes.LocalQuery]: EventDefinition<
    typeof DatabaseEventTypes.LocalQuery,
    DatabaseQueryEventPayload
  >;
  [DatabaseEventTypes.RemoteQuery]: EventDefinition<
    typeof DatabaseEventTypes.RemoteQuery,
    DatabaseQueryEventPayload
  >;
};

export type DatabaseEventSystem = EventSystem<DatabaseEventTypeMap>;

export function createDatabaseEventSystem(): DatabaseEventSystem {
  return createEventSystem([DatabaseEventTypes.LocalQuery, DatabaseEventTypes.RemoteQuery]);
}
