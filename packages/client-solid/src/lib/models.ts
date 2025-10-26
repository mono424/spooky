import { Doc, RecordId, RecordResult } from "surrealdb";

export type GenericModel = Doc;
export type GenericSchema = Record<string, GenericModel>;
export type ModelPayload<T> = RecordResult<Omit<T, "id">>;
export type Model<T> = ModelPayload<T> & { id: RecordId };
