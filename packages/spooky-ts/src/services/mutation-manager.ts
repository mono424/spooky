import {
  SchemaStructure,
  TableModel,
  TableNames,
  GetTable,
  InnerQuery,
  QueryBuilder,
} from "@spooky/query-builder";
import {
  DatabaseService,
  GlobalQueryEventTypes,
  MutationEventTypes,
  SpookyEventSystem,
  TableModelWithId,
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
  selector: InnerQuery<GetTable<S, N>, boolean>;
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
  selector: InnerQuery<GetTable<S, N>, boolean>;
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
  ): Promise<TableModelWithId<GetTable<S, N>>[]> {
    this.logger.debug("[MutationManager] Apply locally", {
      id: mutation.id,
    });

    let rollbackData: TableModelWithId<GetTable<S, N>>[] = [];

    if (mutation.operationType === "create") {
      rollbackData = await this.databaseService.queryLocal<
        TableModelWithId<GetTable<S, N>>[]
      >(`SELECT * FROM ${mutation.recordId.toString()}`);
    } else {
      const selectQuery = mutation.selector.selectQuery;
      rollbackData = await this.databaseService.queryLocal<
        TableModelWithId<GetTable<S, N>>[]
      >(selectQuery.query, selectQuery.vars);
    }

    switch (mutation.operationType) {
      case "create":
        const q = this.buildCreateQuery(mutation.recordId, mutation.data);
        // Always filter out 'id' from variables since it's not in the SET clause
        const { id: _, ...dataWithoutId } = mutation.data as any;
        await this.databaseService.queryLocal<
          TableModelWithId<GetTable<S, N>>[]
        >(q, dataWithoutId);
        break;

      case "update":
        const updateMutation = mutation as UpdateMutation<S, N>;
        const updateQuery = updateMutation.selector.buildUpdateQuery(updateMutation.patches);
        await this.databaseService.queryLocal<
          TableModelWithId<GetTable<S, N>>[]
        >(updateQuery.query, updateQuery.vars);
        break;

      case "delete":
        const deleteMutation = mutation as DeleteMutation<S, N>;
        const deleteQuery = deleteMutation.selector.buildDeleteQuery();
        return await this.databaseService.queryLocal<
          TableModelWithId<GetTable<S, N>>[]
        >(deleteQuery.query, deleteQuery.vars);
        break;

      default:
        throw new Error(`Unknown mutation type`);
    }
    return rollbackData;
  }

  private async mutationApplyRemoteInner<N extends TableNames<S>>(
    mutation: Mutation<S, N>
  ): Promise<TableModelWithId<GetTable<S, N>>[]> {
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
        return this.databaseService.queryRemote<
          TableModelWithId<GetTable<S, N>>[]
        >(q, dataWithoutId);

      case "update":
        const updateQuery = mutation.selector.buildUpdateQuery(mutation.patches);
        this.logger.debug("[MutationManager] Update remote", {
          id: mutation.id,
          query: updateQuery.query,
          data: mutation.patches,
        });
        return this.databaseService.queryRemote<
          TableModelWithId<GetTable<S, N>>[]
        >(
          updateQuery.query,
          updateQuery.vars
        );

      case "delete":
        const deleteQuery = mutation.selector.buildDeleteQuery();
        this.logger.debug("[MutationManager] Delete remote", {
          id: mutation.id,
          query: deleteQuery.query,
        });
        return this.databaseService.queryRemote<
          TableModelWithId<GetTable<S, N>>[]
        >(deleteQuery.query, deleteQuery.vars);

      default:
        throw new Error(`Unknown mutation type`);
    }
  }

  private async mutationApplyRemote<N extends TableNames<S>>(
    mutation: Mutation<S, N>
  ): Promise<TableModelWithId<GetTable<S, N>> | null> {
    this.logger.debug("[MutationManager] Apply remote", {
      id: mutation.id,
    });

    const result: TableModelWithId<GetTable<S, N>>[] =
      await this.mutationApplyRemoteInner(mutation);

    this.logger.debug("[MutationManager] Apply remote - Result", {
      result: result,
    });

    if (result.length > 0) {
      return result[0];
    }

    throw new Error("No result from remote mutation");
  }

  private async executeMutation<N extends TableNames<S>>(
    mutation: Mutation<S, N>
  ): Promise<void> {
    this.logger.debug("[MutationManager] Request execution", {
      mutationId: mutation.id?.toString(),
    });

    let rollbackData: TableModelWithId<GetTable<S, N>> | null = null;
    try {
      try {
        const result = await this.mutationApplyLocal(mutation);
        if (result.length > 0) {
          rollbackData = result[0];
        }
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
        const remoteResult = await this.mutationApplyRemote(mutation);
        if (remoteResult) {
          this.eventSystem.addEvent({
            type: GlobalQueryEventTypes.MaterializeRemoteRecordUpdate,
            payload: {
              record: remoteResult,
            },
          });
        }
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

      if (rollbackData) {
        this.logger.error("[MutationManager] Rolling back mutation", {
          rollbackData,
        });
        this.eventSystem.addEvent({
          type: GlobalQueryEventTypes.MaterializeRemoteRecordUpdate,
          payload: {
            record: rollbackData,
          },
        });
      }
      throw error;
    }
  }

  async create<N extends TableNames<S>>(
    tableName: N,
    payload: TableModel<GetTable<S, N>> & { id?: string | RecordId }
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
    selector: string | InnerQuery<GetTable<S, N>, boolean>,
    payload: Partial<TableModel<GetTable<S, N>>>
  ): Promise<void> {
    let innerQuery: InnerQuery<GetTable<S, N>, boolean>;
    let targetTableName: string = tableName;
    let actualPayload: Partial<TableModel<GetTable<S, N>>> = payload;

    // Handle case where update is called with (recordId, payload)
    // In this case, tableName is recordId, selector is payload, and payload is undefined
    if (!payload && typeof selector === "object" && !(selector instanceof InnerQuery) && typeof tableName === "string" && (tableName as string).includes(":")) {
      const parts = (tableName as string).split(":");
      targetTableName = parts[0];
      actualPayload = selector as unknown as Partial<TableModel<GetTable<S, N>>>;

      innerQuery = new QueryBuilder(this.schema, targetTableName)
        .where({ id: tableName } as any)
        .build().innerQuery;
    } else if (typeof selector === "string") {
      innerQuery = new QueryBuilder(this.schema, tableName)
        .where({ id: selector } as any)
        .build().innerQuery;
    } else {
      innerQuery = selector as InnerQuery<GetTable<S, N>, boolean>;
    }

    if (!innerQuery) {
      throw new Error(`Invalid selector for update operation. tableName: ${tableName}`);
    }

    const patches = Object.entries(actualPayload)
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
          selector: innerQuery,
          tableName: targetTableName as N,
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
    selector: string | InnerQuery<GetTable<S, N>, boolean>
  ): Promise<void> {
    let innerQuery: InnerQuery<GetTable<S, N>, boolean>;
    let targetTableName: string = tableName;

    // Handle case where delete is called with a single argument (recordId)
    // In this case, tableName contains the recordId and selector is undefined
    if (!selector && typeof tableName === "string" && (tableName as string).includes(":")) {
      const parts = (tableName as string).split(":");
      targetTableName = parts[0];

      innerQuery = new QueryBuilder(this.schema, targetTableName)
        .where({ id: tableName } as any)
        .build().innerQuery;
    } else if (typeof selector === "string") {
      innerQuery = new QueryBuilder(this.schema, tableName)
        .where({ id: selector } as any)
        .build().innerQuery;
    } else {
      innerQuery = selector;
    }

    if (!innerQuery) {
      throw new Error(`Invalid selector for delete operation. tableName: ${tableName}, selector: ${selector}`);
    }

    return this.eventSystem.addEvent({
      type: MutationEventTypes.RequestExecution,
      payload: {
        mutation: {
          operationType: "delete",
          tableName: targetTableName as N,
          selector: innerQuery,
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
