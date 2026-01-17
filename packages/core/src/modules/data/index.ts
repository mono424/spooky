// Export unified DataManager
export { DataManager } from './manager.js';

// Export Query types and events
export { QueryManager } from './query.js';
export { Incantation } from './incantation.js';
export { QueryEventTypes, createQueryEventSystem } from './events/query.js';
export type { QueryEventSystem } from './events/query.js';

// Export Mutation types and events
export { MutationManager } from './mutation.js';
export { MutationEventTypes, createMutationEventSystem } from './events/mutation.js';
export type { MutationEventSystem } from './events/mutation.js';
