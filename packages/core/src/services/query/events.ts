import { RecordId, Duration } from 'surrealdb';
import { createEventSystem, EventDefinition, EventSystem } from '../../events/index.js';
import { QueryTimeToLive } from '../../types.js';

export const QueryEventTypes = {
  IncantationInitialized: 'QUERY_INCANTATION_INITIALIZED',
  IncantationRemoteHashUpdate: 'QUERY_INCANTATION_REMOTE_HASH_UPDATE',
  IncantationTTLHeartbeat: 'QUERY_INCANTATION_TTL_HEARTBEAT',
  IncantationCleanup: 'QUERY_INCANTATION_CLEANUP',

  // Sent from sync service
  IncantationIncomingRemoteUpdate: 'QUERY_INCANTATION_INCOMING_REMOTE_UPDATE',

  // Outside events
  IncantationUpdated: 'QUERY_INCANTATION_UPDATED',
} as const;

export type QueryEventTypeMap = {
  [QueryEventTypes.IncantationInitialized]: EventDefinition<
    typeof QueryEventTypes.IncantationInitialized,
    {
      incantationId: RecordId<string>;
      surrealql: string;
      params: Record<string, any>;
      ttl: QueryTimeToLive | Duration;
    }
  >;
  [QueryEventTypes.IncantationRemoteHashUpdate]: EventDefinition<
    typeof QueryEventTypes.IncantationRemoteHashUpdate,
    {
      incantationId: RecordId<string>;
      surrealql: string;
      params: Record<string, any>;
      localHash: string;
      localTree: any;
      remoteHash: string;
      remoteTree: any;
    }
  >;
  [QueryEventTypes.IncantationTTLHeartbeat]: EventDefinition<
    typeof QueryEventTypes.IncantationTTLHeartbeat,
    {
      incantationId: RecordId<string>;
    }
  >;

  [QueryEventTypes.IncantationCleanup]: EventDefinition<
    typeof QueryEventTypes.IncantationCleanup,
    {
      incantationId: RecordId<string>;
    }
  >;

  [QueryEventTypes.IncantationIncomingRemoteUpdate]: EventDefinition<
    typeof QueryEventTypes.IncantationIncomingRemoteUpdate,
    {
      incantationId: RecordId<string>;
      remoteHash: string;
      remoteTree: any;
      records: Record<string, any>[];
    }
  >;

  [QueryEventTypes.IncantationUpdated]: EventDefinition<
    typeof QueryEventTypes.IncantationUpdated,
    {
      incantationId: RecordId<string>;
      records: Record<string, any>[];
      tree?: any;
    }
  >;
};

export type QueryEventSystem = EventSystem<QueryEventTypeMap>;

export function createQueryEventSystem(): QueryEventSystem {
  return createEventSystem([
    QueryEventTypes.IncantationInitialized,
    QueryEventTypes.IncantationRemoteHashUpdate,
    QueryEventTypes.IncantationCleanup,
    QueryEventTypes.IncantationIncomingRemoteUpdate,
    QueryEventTypes.IncantationUpdated,
    QueryEventTypes.IncantationTTLHeartbeat,
  ]);
}
