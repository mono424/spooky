import {
  SchemaStructure,
  TableModel,
  TableNames,
  GetTable,
} from "@spooky/query-builder";
import {
  DatabaseService,
  GlobalQueryEventTypes,
  MutationEventTypes,
  SpookyEventSystem,
} from "./index.js";
import { encodeToSpooky, generateNewId } from "./converter.js";
import { RecordId } from "surrealdb";
import { Logger } from "./logger.js";

export type MutationType = "create" | "update" | "delete";

export type Mutation<S extends SchemaStructure, N extends TableNames<S>> =
  | CreateMutation<S, N>
  | UpdateMutation<S, N>
  | DeleteMutation<S, N>;

export interface JsonPatch {
  op: string;
  path: string;
  value?: unknown;
}

export interface CreateMutation<
  S extends SchemaStructure,
  N extends TableNames<S>
> {
  id?: RecordId;
  operationType: "create";
  tableName: N;
  recordId: RecordId;
  data: any;
  createdAt: Date;
  retryCount: number;
  lastError?: string;
}

export interface UpdateMutation<
  S extends SchemaStructure,
  N extends TableNames<S>
> {
  id?: RecordId;
  operationType: "update";
  tableName: N;
  recordId: RecordId;
  patches: JsonPatch[];
  rollbackPatches: JsonPatch[] | null;
  createdAt: Date;
  retryCount: number;
  lastError?: string;
}

export interface DeleteMutation<
  S extends SchemaStructure,
  N extends TableNames<S>
> {
  id?: RecordId;
  operationType: "delete";
  tableName: N;
  recordId: RecordId;
  rollbackData: TableModel<GetTable<S, N>> | null;
  createdAt: Date;
  retryCount: number;
  lastError?: string;
}

export class MutationManagerService<S extends SchemaStructure> {
  constructor(
    private schema: S,
    private databaseService: DatabaseService,
    private logger: Logger,
    private eventSystem: SpookyEventSystem
  ) {
    this.eventSystem.subscribe(MutationEventTypes.RequestExecution, (event) => {
      this.executeMutation(
        event.payload.mutation as Mutation<S, TableNames<S>>
      );
    });
  }

  private buildRecordId(table: string, id: string): RecordId {
    if (id.includes(":")) {
      const [table, ...idParts] = id.split(":");
      return new RecordId(table, idParts.join(":"));
    }
    return new RecordId(table, id);
  }

  private buildCreateQuery<N extends TableNames<S>>(
    id: RecordId,
    payload: TableModel<GetTable<S, N>>
  ): string {
    // Always exclude 'id' from SET clause since it's specified in CREATE statement
    const setQuery = Object.entries(payload)
      .filter(([key]) => key !== "id")
      .map(([key, value]) => `${key} = $${key}`)
      .join(", ");

    // In surrealdb 1.x, RecordId has .tb and .id properties
    return `CREATE ${id.tb}:${id.id} SET ${setQuery};`;
  }

  private async mutationApplyLocal<N extends TableNames<S>>(
    mutation: Mutation<S, N>
  ): Promise<void> {
    this.logger.debug("[MutationManager] Apply locally", {
      id: mutation.id,
    });

    switch (mutation.operationType) {
      case "create":
        const q = this.buildCreateQuery(mutation.recordId, mutation.data);
        // Always filter out 'id' from variables since it's not in the SET clause
        const { id: _, ...dataWithoutId } = mutation.data as any;
        await this.databaseService.queryLocal<TableModel<GetTable<S, N>>[]>(
          q,
          dataWithoutId
        );
        break;

      case "update":
        const updateMutation = mutation as UpdateMutation<S, N>;
        await this.databaseService.queryLocal<TableModel<GetTable<S, N>>[]>(
          `UPDATE ${updateMutation.recordId.toString()} CONTENT $patches`,
          {
            patches: updateMutation.patches,
          }
        );
        break;

      case "delete":
        const deleteMutation = mutation as DeleteMutation<S, N>;
        await this.databaseService.queryLocal<TableModel<GetTable<S, N>>[]>(
          `DELETE ${deleteMutation.recordId.toString()}`
        );
        break;

      default:
        throw new Error(`Unknown mutation type`);
    }

    this.logger.debug("[MutationManager] Apply locally - Done", {
      id: mutation.id,
    });
  }

  private async mutationApplyRemote<N extends TableNames<S>>(
    mutation: Mutation<S, N>
  ): Promise<void> {
    this.logger.debug("[MutationManager] Apply remote", {
      id: mutation.id,
    });

    switch (mutation.operationType) {
      case "create":
        const q = this.buildCreateQuery(mutation.recordId, mutation.data);
        // Always filter out 'id' from variables since it's not in the SET clause
        const { id: _, ...dataWithoutId } = mutation.data as any;
        this.logger.debug("[MutationManager] Create remote", {
          id: mutation.id,
          query: q,
          data: dataWithoutId,
        });
        await this.databaseService.queryRemote<TableModel<GetTable<S, N>>[]>(
          q,
          dataWithoutId
        );
        break;

      case "update":
        this.logger.debug("[MutationManager] Update remote", {
          id: mutation.id,
          recordId: mutation.recordId,
          data: mutation.patches,
        });
        await this.databaseService.queryRemote<TableModel<GetTable<S, N>>[]>(
          `UPDATE ${mutation.recordId.toString()} CONTENT $patches`,
          {
            patches: mutation.patches,
          }
        );
        break;

      case "delete":
        this.logger.debug("[MutationManager] Delete remote", {
          id: mutation.id,
          recordId: mutation.recordId,
        });
        await this.databaseService.queryRemote<TableModel<GetTable<S, N>>[]>(
          `DELETE ${mutation.recordId.toString()}`
        );
        break;

      default:
        throw new Error(`Unknown mutation type`);
    }

    this.logger.debug("[MutationManager] Apply remote - Done", {
      id: mutation.id,
    });
  }

  private async executeMutation<N extends TableNames<S>>(
    mutation: Mutation<S, N>
  ): Promise<void> {
    this.logger.debug("[MutationManager] Request execution", {
      mutationId: mutation.id?.toString(),
    });
    try {
      try {
        await this.mutationApplyLocal(mutation);
      } catch (error) {
        this.logger.error("Failed to apply mutation locally", error);
        throw error;
      }

      this.eventSystem.addEvent({
        type: GlobalQueryEventTypes.RequestTableQueryRefresh,
        payload: {
          table: mutation.tableName,
        },
      });

      try {
        await this.mutationApplyRemote(mutation);
      } catch (error) {
        this.logger.error("Failed to apply mutation remotely", error);
        throw error;
      }

      this.logger.debug(
        "[MutationManager] Execute mutation - Successful",
        mutation
      );
    } catch (error) {
      this.logger.error("[MutationManager] Execute mutation - Failed", error);
      throw error;
    } finally {
      this.logger.debug(
        "[MutationManager] Execute mutation - Refreshing table queries",
        {
          table: mutation.tableName,
        }
      );
      this.eventSystem.addEvent({
        type: GlobalQueryEventTypes.RequestTableQueryRefresh,
        payload: {
          table: mutation.tableName,
        },
      });
    }
  }

  async create<N extends TableNames<S>>(
    tableName: N,
    payload: TableModel<GetTable<S, N>>
  ): Promise<void> {
    const encodedPayload = encodeToSpooky(this.schema, tableName, payload);
    if (!encodedPayload) {
      throw new Error("payload could not be encoded");
    }

    // Check if payload has an 'id' field
    const hasId =
      encodedPayload &&
      typeof encodedPayload === "object" &&
      "id" in encodedPayload &&
      encodedPayload.id != null;

    let id: RecordId;
    if (hasId) {
      // Use the ID from the payload
      const payloadId = (encodedPayload as any).id;
      if (payloadId instanceof RecordId) {
        id = payloadId;
      } else if (typeof payloadId === "string") {
        // Parse string format "table:id"
        const [tb, ...idParts] = payloadId.split(":");
        id = new RecordId(tb, idParts.join(":"));
      } else {
        throw new Error("Invalid id format in payload");
      }
    } else {
      // Generate a new ID
      id = generateNewId(tableName);
      if (!id) {
        throw new Error("id could not be generated");
      }
    }

    return this.eventSystem.addEvent({
      type: MutationEventTypes.RequestExecution,
      payload: {
        mutation: {
          operationType: "create",
          tableName: tableName,
          recordId: id,
          data: encodedPayload,
          createdAt: new Date(),
          retryCount: 0,
        },
      },
    });
  }

  async update<N extends TableNames<S>>(
    tableName: N,
    recordId: string,
    payload: Partial<TableModel<GetTable<S, N>>>
  ): Promise<void> {
    const patches = Object.entries(payload)
      .filter(([key]) => key !== "id")
      .map(([key, value]) => ({
        op: "replace",
        path: `/${key}`,
        value: value,
      }));

    return this.eventSystem.addEvent({
      type: MutationEventTypes.RequestExecution,
      payload: {
        mutation: {
          operationType: "update",
          recordId: this.buildRecordId(tableName, recordId),
          tableName: tableName,
          patches: patches,
          rollbackPatches: null,
          createdAt: new Date(),
          retryCount: 0,
        },
      },
    });
  }

  async delete<N extends TableNames<S>>(
    tableName: N,
    id: string
  ): Promise<void> {
    return this.eventSystem.addEvent({
      type: MutationEventTypes.RequestExecution,
      payload: {
        mutation: {
          operationType: "delete",
          tableName: tableName,
          recordId: this.buildRecordId(tableName, id),
          rollbackData: null,
          createdAt: new Date(),
          retryCount: 0,
        },
      },
    });
  }
}

export function createMutationManagerService<S extends SchemaStructure>(
  schema: S,
  databaseService: DatabaseService,
  logger: Logger,
  eventSystem: SpookyEventSystem
): MutationManagerService<S> {
  return new MutationManagerService(
    schema,
    databaseService,
    logger,
    eventSystem
  );
}
