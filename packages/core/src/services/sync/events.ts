import { RecordId } from "surrealdb";
import {
  createEventSystem,
  EventDefinition,
  EventSystem,
} from "../../events/index.js";
import { QueryTimeToLive } from "../../types.js";

export const SyncQueueEventTypes = {
  MutationEnqueued: "MUTATION_ENQUEUED",
  IncantationRegistrationEnqueued: "INCANTATION_REGISTRATION_ENQUEUED",
  IncantationSyncEnqueued: "INCANTATION_SYNC_ENQUEUED",
  IncantationTTLHeartbeatEnqueued: "INCANTATION_TTL_HEARTBEAT_ENQUEUED",
  IncantationCleanupEnqueued: "INCANTATION_CLEANUP_ENQUEUED",
} as const;

export type SyncQueueEventTypeMap = {
  [SyncQueueEventTypes.MutationEnqueued]: EventDefinition<
    typeof SyncQueueEventTypes.MutationEnqueued,
    { queueSize: number }
  >;
  [SyncQueueEventTypes.IncantationRegistrationEnqueued]: EventDefinition<
    typeof SyncQueueEventTypes.IncantationRegistrationEnqueued,
    { incantationId: RecordId<string>, surrealql: string, ttl: QueryTimeToLive }
  >;
  [SyncQueueEventTypes.IncantationSyncEnqueued]: EventDefinition<
    typeof SyncQueueEventTypes.IncantationSyncEnqueued,
    { incantationId: RecordId<string>, remoteHash: string }
  >;
  [SyncQueueEventTypes.IncantationTTLHeartbeatEnqueued]: EventDefinition<
    typeof SyncQueueEventTypes.IncantationTTLHeartbeatEnqueued,
    { incantationId: RecordId<string> }
  >;
  [SyncQueueEventTypes.IncantationCleanupEnqueued]: EventDefinition<
    typeof SyncQueueEventTypes.IncantationCleanupEnqueued,
    { incantationId: RecordId<string> }
  >;
};

export type SyncQueueEventSystem = EventSystem<SyncQueueEventTypeMap>;

export function createSyncQueueEventSystem(): SyncQueueEventSystem {
  return createEventSystem([
    SyncQueueEventTypes.MutationEnqueued,
    SyncQueueEventTypes.IncantationRegistrationEnqueued,
    SyncQueueEventTypes.IncantationSyncEnqueued,
    SyncQueueEventTypes.IncantationTTLHeartbeatEnqueued,
    SyncQueueEventTypes.IncantationCleanupEnqueued,
  ]);
}
