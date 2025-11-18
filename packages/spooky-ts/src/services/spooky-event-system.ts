import { RecordId } from "@spooky/query-builder";
import {
  createEventSystem,
  EventDefinition,
  EventSystem,
} from "../events/index.js";

export const AuthEventTypes = {
  Authenticated: "AUTHENTICATED",
  Deauthenticated: "DEAUTHENTICATED",
} as const;

export const QueryEventTypes = {
  RequestInit: "QUERY_REQUEST_INIT",
  Updated: "QUERY_UPDATED",
  RemoteUpdate: "QUERY_REMOTE_UPDATE",
} as const;

export type SpookyEventTypeMap = {
  [AuthEventTypes.Authenticated]: EventDefinition<
    typeof AuthEventTypes.Authenticated,
    {
      userId: RecordId;
      token: string;
    }
  >;
  [AuthEventTypes.Deauthenticated]: EventDefinition<
    typeof AuthEventTypes.Deauthenticated,
    never
  >;
  [QueryEventTypes.RequestInit]: EventDefinition<
    typeof QueryEventTypes.RequestInit,
    {
      queryHash: number;
    }
  >;
  [QueryEventTypes.Updated]: EventDefinition<
    typeof QueryEventTypes.Updated,
    {
      queryHash: number;
      data: Record<string, unknown>[];
    }
  >;
  [QueryEventTypes.RemoteUpdate]: EventDefinition<
    typeof QueryEventTypes.RemoteUpdate,
    {
      queryHash: number;
      data: Record<string, unknown>[];
    }
  >;
};

export type SpookyEventSystem = EventSystem<SpookyEventTypeMap>;

export function createSpookyEventSystem(): SpookyEventSystem {
  return createEventSystem([
    AuthEventTypes.Authenticated,
    AuthEventTypes.Deauthenticated,
    QueryEventTypes.RequestInit,
    QueryEventTypes.Updated,
    QueryEventTypes.RemoteUpdate,
  ]);
}
