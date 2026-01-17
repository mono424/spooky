import { createEventSystem, EventDefinition, EventSystem } from '../../../events/index.js';
import { UpEvent } from '../../sync/queue/queue-up.js';

export const MutationEventTypes = {
  MutationCreated: 'MUTATION_CREATED',
} as const;

export type MutationEventTypeMap = {
  [MutationEventTypes.MutationCreated]: EventDefinition<
    typeof MutationEventTypes.MutationCreated,
    UpEvent[]
  >;
};

export type MutationEventSystem = EventSystem<MutationEventTypeMap>;

export function createMutationEventSystem(): MutationEventSystem {
  return createEventSystem([MutationEventTypes.MutationCreated]);
}
