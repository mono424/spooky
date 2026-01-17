import { RecordId, Duration } from 'surrealdb';
import { createEventSystem, EventDefinition, EventSystem } from '../../../events/index.js';
import { QueryTimeToLive, RecordVersionArray } from '../../../types.js';

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
      localHash?: string;
      localArray?: RecordVersionArray;
      records?: Record<string, any>[];
    }
  >;
  [QueryEventTypes.IncantationRemoteHashUpdate]: EventDefinition<
    typeof QueryEventTypes.IncantationRemoteHashUpdate,
    {
      incantationId: RecordId<string>;
      surrealql: string;
      params: Record<string, any>;
      localHash: string;
      localArray: RecordVersionArray;
      remoteHash: string;
      remoteArray: RecordVersionArray;
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
      remoteArray: RecordVersionArray;
      records: Record<string, any>[];
    }
  >;

  [QueryEventTypes.IncantationUpdated]: EventDefinition<
    typeof QueryEventTypes.IncantationUpdated,
    {
      incantationId: RecordId<string>;
      records: Record<string, any>[];
      array?: RecordVersionArray;
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
