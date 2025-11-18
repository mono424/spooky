import {
  SchemaStructure,
  TableModel,
  TableNames,
  GetTable,
} from "@spooky/query-builder";
import { DatabaseService } from "./index.js";
import { encodeToSpooky, generateNewId } from "./converter.js";
import { RecordId } from "surrealdb";
import { Logger } from "./logger.js";
import { QueryManagerService } from "./query-manager.js";

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
    private queryManagerService: QueryManagerService<S>,
    private logger: Logger
  ) {}

  private buildCreateQuery<N extends TableNames<S>>(
    id: RecordId,
    payload: TableModel<GetTable<S, N>>
  ): string {
    const setQuery = Object.entries(payload)
      .map(([key, value]) => `${key} = $${key}`)
      .join(", ");

    // In surrealdb 1.x, RecordId has .tb and .id properties
    return `CREATE ${id.tb}:${id.id} SET ${setQuery};`;
  }

  private async createMutation<N extends TableNames<S>>(
    mutation: Mutation<S, N>
  ): Promise<Mutation<S, N>> {
    const [result] = await this.databaseService.queryInternal<{ id: RecordId }[]>(
      `CREATE _mutations CONTENT $payload`,
      {
        payload: mutation,
      }
    );
    return {
      ...mutation,
      id: result.id,
    };
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
        await this.databaseService.queryLocal<TableModel<GetTable<S, N>>[]>(
          q,
          mutation.data
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
        this.logger.debug("[MutationManager] Create remote", {
          id: mutation.id,
          query: q,
          data: mutation.data,
        });
        await this.databaseService.queryRemote<TableModel<GetTable<S, N>>[]>(
          q,
          mutation.data
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

  private async runWithRetry<N extends TableNames<S>>(
    mutation: Mutation<S, N>
  ): Promise<void> {
    const maxRetries = 3;
    const baseDelay = 10;

    for (let attempt = 0; attempt <= maxRetries; attempt++) {
      try {
        await this.mutationApplyRemote(mutation);
        return; // Success, exit the retry loop
      } catch (error) {
        if (attempt === maxRetries) {
          this.logger.error("Failed to apply mutation remotely", error);
          throw error;
        }
        // Exponential backoff
        const delay = baseDelay * Math.pow(2, attempt);
        await new Promise((resolve) => setTimeout(resolve, delay));
      }
    }
  }

  private async run<N extends TableNames<S>>(
    mutation: Mutation<S, N>
  ): Promise<void> {
    this.logger.debug("[MutationManager] Run - Starting");

    try {
      await this.createMutation(mutation);
    } catch (error) {
      this.logger.error("Failed to create mutation", error);
    }

    try {
      await this.mutationApplyLocal(mutation);
    } catch (error) {
      this.logger.error("Failed to apply mutation locally", error);
    }

    await this.queryManagerService.refreshTableQueries(mutation.tableName);

    try {
      await this.runWithRetry(mutation);
    } catch (error) {
      this.logger.error("Failed to apply mutation remotely", error);
    }

    await this.queryManagerService.refreshTableQueries(mutation.tableName);

    this.logger.debug("[MutationManager] Run - Done");
  }

  async create<N extends TableNames<S>>(
    tableName: N,
    payload: TableModel<GetTable<S, N>>
  ): Promise<void> {
    const encodedPayload = encodeToSpooky(this.schema, tableName, payload);
    if (!encodedPayload) {
      throw new Error("payload could not be encoded");
    }

    const id = generateNewId(tableName);
    if (!id) {
      throw new Error("id could not be generated");
    }

    return this.run<N>({
      operationType: "create",
      tableName: tableName,
      recordId: id,
      data: encodedPayload,
      createdAt: new Date(),
      retryCount: 0,
    });
  }

  async update<N extends TableNames<S>>(
    tableName: N,
    recordId: RecordId,
    payload: Partial<TableModel<GetTable<S, N>>>
  ): Promise<void> {
    const patches = Object.entries(payload)
      .filter(([key]) => key !== "id")
      .map(([key, value]) => ({
        op: "replace",
        path: `/${key}`,
        value: value,
      }));

    return this.run<N>({
      operationType: "update",
      recordId,
      tableName: tableName,
      patches: patches,
      rollbackPatches: null,
      createdAt: new Date(),
      retryCount: 0,
    });
  }

  async delete<N extends TableNames<S>>(
    tableName: N,
    id: RecordId
  ): Promise<void> {
    return this.run<N>({
      operationType: "delete",
      tableName: tableName,
      recordId: id,
      rollbackData: null,
      createdAt: new Date(),
      retryCount: 0,
    });
  }
}

export function createMutationManagerService<S extends SchemaStructure>(
  schema: S,
  databaseService: DatabaseService,
  queryManagerService: QueryManagerService<S>,
  logger: Logger
): MutationManagerService<S> {
  return new MutationManagerService(
    schema,
    databaseService,
    queryManagerService,
    logger
  );
}
