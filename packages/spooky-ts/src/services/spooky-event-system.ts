import {
  ColumnSchema,
  RecordId,
  SchemaStructure,
  TableNames,
} from "@spooky/query-builder";
import {
  createEventSystem,
  EventDefinition,
  EventSystem,
} from "../events/index.js";
import { Mutation } from "./mutation-manager.js";
import { TableModelWithId } from "./query-manager.js";

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
  RequestTableQueryRefresh: "QUERY_REQUEST_TABLE_QUERY_REFRESH",
  MaterializeRemoteRecordUpdate: "QUERY_MATERIALIZE_REMOTE_RECORD_UPDATE",
} as const;

export const MutationEventTypes = {
  RequestExecution: "MUTATION_REQUEST_EXECUTION",
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
      dataHash: number;
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
  [MutationEventTypes.RequestExecution]: EventDefinition<
    typeof MutationEventTypes.RequestExecution,
    {
      mutation: Mutation<SchemaStructure, TableNames<SchemaStructure>>;
    }
  >;
  [GlobalQueryEventTypes.RequestTableQueryRefresh]: EventDefinition<
    typeof GlobalQueryEventTypes.RequestTableQueryRefresh,
    {
      table: string;
    }
  >;
  [GlobalQueryEventTypes.MaterializeRemoteRecordUpdate]: EventDefinition<
    typeof GlobalQueryEventTypes.MaterializeRemoteRecordUpdate,
    {
      record: TableModelWithId<{ columns: Record<string, ColumnSchema> }>;
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
    GlobalQueryEventTypes.RequestTableQueryRefresh,
    MutationEventTypes.RequestExecution,
    GlobalQueryEventTypes.MaterializeRemoteRecordUpdate,
  ]);
}
