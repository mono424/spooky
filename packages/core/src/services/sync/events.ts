import { createEventSystem, EventDefinition, EventSystem } from '../../events/index.js';

export const SyncQueueEventTypes = {
  MutationEnqueued: 'MUTATION_ENQUEUED',
  QueryItemEnqueued: 'QUERY_ITEM_ENQUEUED',
} as const;

export type SyncQueueEventTypeMap = {
  [SyncQueueEventTypes.MutationEnqueued]: EventDefinition<
    typeof SyncQueueEventTypes.MutationEnqueued,
    { queueSize: number }
  >;
  [SyncQueueEventTypes.QueryItemEnqueued]: EventDefinition<
    typeof SyncQueueEventTypes.QueryItemEnqueued,
    { queueSize: number }
  >;
};

export type SyncQueueEventSystem = EventSystem<SyncQueueEventTypeMap>;

export function createSyncQueueEventSystem(): SyncQueueEventSystem {
  return createEventSystem([
    SyncQueueEventTypes.QueryItemEnqueued,
    SyncQueueEventTypes.MutationEnqueued,
  ]);
}

export const SyncEventTypes = {
  IncantationUpdated: 'SYNC_INCANTATION_UPDATED',
} as const;

export type SyncEventTypeMap = {
  [SyncEventTypes.IncantationUpdated]: EventDefinition<
    typeof SyncEventTypes.IncantationUpdated,
    {
      incantationId: any; // RecordId<string> but imported
      remoteHash: string;
      remoteTree: any;
      records: Record<string, any>[];
    }
  >;
};

export type SyncEventSystem = EventSystem<SyncEventTypeMap>;

export function createSyncEventSystem(): SyncEventSystem {
  return createEventSystem([SyncEventTypes.IncantationUpdated]);
}
