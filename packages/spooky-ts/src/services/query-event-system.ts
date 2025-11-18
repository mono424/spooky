import {
  createEventSystem,
  EventDefinition,
  EventSystem,
} from "../events/index.js";

export const QueryEventTypes = {
  Updated: "QUERY_UPDATED",
  Destroyed: "QUERY_DESTROYED",
} as const;

export type QueryEventTypeMap = {
  [QueryEventTypes.Updated]: EventDefinition<
    typeof QueryEventTypes.Updated,
    {
      type: "local" | "remote";
      data: Record<string, unknown>[];
    }
  >;
  [QueryEventTypes.Destroyed]: EventDefinition<
    typeof QueryEventTypes.Destroyed,
    {
      queryHash: number;
    }
  >;
};

export type QueryEventSystem = EventSystem<QueryEventTypeMap>;

export function createQueryEventSystem(): QueryEventSystem {
  return createEventSystem([
    QueryEventTypes.Updated,
    QueryEventTypes.Destroyed,
  ]);
}
