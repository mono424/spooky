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

export const GlobalQueryEventTypes = {
  SubqueryUpdated: "SUBQUERY_UPDATED",
  RequestInit: "QUERY_REQUEST_INIT",
  Updated: "QUERY_UPDATED",
  RemoteUpdate: "QUERY_REMOTE_UPDATE",
  Destroyed: "QUERY_DESTROYED",
  RemoteLiveUpdate: "QUERY_REMOTE_LIVE_UPDATE",
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
  [GlobalQueryEventTypes.RequestInit]: EventDefinition<
    typeof GlobalQueryEventTypes.RequestInit,
    {
      queryHash: number;
      query?: string;
      variables?: Record<string, unknown>;
    }
  >;
  [GlobalQueryEventTypes.Updated]: EventDefinition<
    typeof GlobalQueryEventTypes.Updated,
    {
      queryHash: number;
      data: Record<string, unknown>[];
    }
  >;
  [GlobalQueryEventTypes.RemoteUpdate]: EventDefinition<
    typeof GlobalQueryEventTypes.RemoteUpdate,
    {
      queryHash: number;
      data: Record<string, unknown>[];
    }
  >;
  [GlobalQueryEventTypes.Destroyed]: EventDefinition<
    typeof GlobalQueryEventTypes.Destroyed,
    {
      queryHash: number;
    }
  >;
  [GlobalQueryEventTypes.SubqueryUpdated]: EventDefinition<
    typeof GlobalQueryEventTypes.SubqueryUpdated,
    {
      queryHash: number;
      subqueryHash: number;
    }
  >;
  [GlobalQueryEventTypes.RemoteLiveUpdate]: EventDefinition<
    typeof GlobalQueryEventTypes.RemoteLiveUpdate,
    {
      queryHash: number;
      action: "CREATE" | "UPDATE" | "DELETE" | "CLOSE";
      update: Record<string, unknown>;
    }
  >;
};

export type SpookyEventSystem = EventSystem<SpookyEventTypeMap>;

export function createSpookyEventSystem(): SpookyEventSystem {
  return createEventSystem([
    AuthEventTypes.Authenticated,
    AuthEventTypes.Deauthenticated,
    GlobalQueryEventTypes.RequestInit,
    GlobalQueryEventTypes.Updated,
    GlobalQueryEventTypes.RemoteUpdate,
    GlobalQueryEventTypes.Destroyed,
    GlobalQueryEventTypes.SubqueryUpdated,
    GlobalQueryEventTypes.RemoteLiveUpdate,
  ]);
}
