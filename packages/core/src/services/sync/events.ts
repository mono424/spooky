import {
  createEventSystem,
  EventDefinition,
  EventSystem,
} from "../../events/index.js";

export const SyncQueueEventTypes = {
  MutationEnqueued: "MUTATION_ENQUEUED",
} as const;

export type SyncQueueEventTypeMap = {
  [SyncQueueEventTypes.MutationEnqueued]: EventDefinition<
    typeof SyncQueueEventTypes.MutationEnqueued,
    { queueSize: number }
  >;
};

export type SyncQueueEventSystem = EventSystem<SyncQueueEventTypeMap>;

export function createSyncQueueEventSystem(): SyncQueueEventSystem {
  return createEventSystem([
    SyncQueueEventTypes.MutationEnqueued,
  ]);
}
