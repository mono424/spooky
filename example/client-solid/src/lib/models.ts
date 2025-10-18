import { Doc, RecordResult } from "surrealdb";

export type GenericModel = Doc;
export type GenericSchema = Record<string, GenericModel>;
export type Model<T> = RecordResult<Omit<T, "id">>;
