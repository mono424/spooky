import { RecordId } from "surrealdb";

// Re-export types from query-builder for backward compatibility
export type { GenericModel, GenericSchema } from "@spooky/query-builder";

// Model and ModelPayload types for the client
export type Model<T> = T;
export type ModelPayload<T> = T & { id: RecordId };
