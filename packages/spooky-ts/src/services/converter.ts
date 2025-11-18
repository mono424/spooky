import {
  GetTable,
  SchemaStructure,
  TableModel,
  TableNames,
} from "@spooky/query-builder";
import { Uuid, RecordId } from "surrealdb";

export function decodeFromSpooky<
  S extends SchemaStructure,
  T extends TableNames<S>
>(schema: S, tableName: T, record: TableModel<GetTable<S, T>>): TableModel<GetTable<S, T>> {
  const table = schema.tables.find((t) => t.name === tableName);
  if (!table) {
    throw new Error(`Table ${tableName} not found in schema`);
  }

  const encoded = { ...record } as any;

  for (const field of Object.keys(table.columns)) {
    const column = table.columns[field] as any;
    if (column.recordId && encoded[field] != null) {
      const recordId = encoded[field] as RecordId;
      // In surrealdb 1.x, RecordId has .tb and .id properties
      encoded[field] = `${recordId.tb}:${recordId.id}`;
    }
  }

  return encoded as TableModel<GetTable<S, T>>;
}

export function encodeToSpooky<
  S extends SchemaStructure,
  T extends TableNames<S>,
  R extends Partial<TableModel<GetTable<S, T>>>
>(schema: S, tableName: T, record: R): R {
  const table = schema.tables.find((t) => t.name === tableName);
  if (!table) {
    throw new Error(`Table ${tableName} not found in schema`);
  }

  const decoded = { ...record } as any;
  for (const field of Object.keys(table.columns)) {
    const column = table.columns[field] as any;
    
    if (column.recordId && decoded[field] != null) {
      // Handle both string format ("table:id") and RecordId objects
      if (decoded[field] instanceof RecordId) {
        // Already a RecordId, keep it as is
        decoded[field] = decoded[field];
      } else if (typeof decoded[field] === "string") {
        // String format, convert to RecordId
        const [tableName, ...idParts] = decoded[field].split(":");
        decoded[field] = new RecordId(tableName, idParts.join(":"));
      }
      // If it's neither, leave it as is (might be undefined or null)
    }
    
    // Convert datetime strings to Date objects
    if (column.dateTime && decoded[field] != null) {
      if (typeof decoded[field] === "string") {
        // ISO string format, convert to Date object
        decoded[field] = new Date(decoded[field]);
      } else if (decoded[field] instanceof Date) {
        // Already a Date, keep it as is
        decoded[field] = decoded[field];
      }
      // If it's neither, leave it as is (might be undefined or null)
    }
  }

  return decoded;
}

export function generateNewId<
  S extends SchemaStructure,
  T extends TableNames<S>
>(tableName: T): RecordId {
  return new RecordId(tableName, Uuid.v4().toString().replace(/-/g, ""));
}
