import { createEventSystem, EventDefinition, EventSystem } from '../../../events/index';
import { RecordVersionArray } from '../../../types';

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
  QueryUpdated: 'SYNC_QUERY_UPDATED',
  RemoteDataIngested: 'SYNC_REMOTE_DATA_INGESTED',
} as const;

export type SyncEventTypeMap = {
  [SyncEventTypes.QueryUpdated]: EventDefinition<
    typeof SyncEventTypes.QueryUpdated,
    {
      queryId: any; // RecordId<string> but imported
      localHash?: string;
      localArray?: RecordVersionArray;
      remoteHash?: string;
      remoteArray?: RecordVersionArray;
      records: Record<string, any>[];
    }
  >;
  [SyncEventTypes.RemoteDataIngested]: EventDefinition<
    typeof SyncEventTypes.RemoteDataIngested,
    {
      records: Record<string, any>[];
    }
  >;
};

export type SyncEventSystem = EventSystem<SyncEventTypeMap>;

export function createSyncEventSystem(): SyncEventSystem {
  return createEventSystem([SyncEventTypes.QueryUpdated, SyncEventTypes.RemoteDataIngested]);
}
