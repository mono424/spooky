import { RecordId } from "surrealdb";
import {
  createEventSystem,
  EventDefinition,
  EventSystem,
} from "../../events/index.js";
import { QueryTimeToLive } from "../../types.js";

export const QueryEventTypes = {
  IncantationInitialized: "QUERY_INCANTATION_INITIALIZED",
  IncantationRemoteHashUpdate: "QUERY_INCANTATION_REMOTE_HASH_UPDATE",
  IncantationTTLHeartbeat: "QUERY_INCANTATION_TTL_HEARTBEAT",
  IncantationCleanup: "QUERY_INCANTATION_CLEANUP",
} as const;

export type QueryEventTypeMap = {
  [QueryEventTypes.IncantationInitialized]: EventDefinition<
    typeof QueryEventTypes.IncantationInitialized,
    {
      incantationId: RecordId<string>;
      surrealql: string;
      ttl: QueryTimeToLive;
    }
  >,
  [QueryEventTypes.IncantationRemoteHashUpdate]: EventDefinition<
    typeof QueryEventTypes.IncantationRemoteHashUpdate,
    {
      incantationId: RecordId<string>;
      remoteHash: string;
      tree: any;
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
};

export type QueryEventSystem = EventSystem<QueryEventTypeMap>;

export function createQueryEventSystem(): QueryEventSystem {
  return createEventSystem([
    QueryEventTypes.IncantationInitialized,
    QueryEventTypes.IncantationRemoteHashUpdate,
    QueryEventTypes.IncantationCleanup,
  ]);
}
