import {
  GetTable,
  SchemaStructure,
  TableModel,
  TableNames,
} from "@spooky/query-builder";
import { Effect } from "effect";
import { Uuid, RecordId } from "surrealdb";

export const decodeFromSpooky = Effect.fn("decodeFromSpooky")(function* <
  S extends SchemaStructure,
  T extends TableNames<S>
>(schema: S, tableName: T, record: TableModel<GetTable<S, T>>) {
  const table = schema.tables.find((t) => t.name === tableName);
  if (!table) {
    yield* Effect.fail(new Error(`Table ${tableName} not found in schema`));
    return;
  }

  const encoded = { ...record } as any;

  for (const field of Object.keys(table.columns)) {
    const column = table.columns[field] as any;
    if (column.recordId && encoded[field] != null) {
      encoded[field] =
        (encoded[field] as RecordId).table.name +
        ":" +
        (encoded[field] as RecordId).id;
    }
  }

  return encoded as TableModel<GetTable<S, T>>;
});

export const encodeToSpooky = Effect.fn("encodeToSpooky")(function* <
  S extends SchemaStructure,
  T extends TableNames<S>,
  R extends Partial<TableModel<GetTable<S, T>>>
>(schema: S, tableName: T, record: R) {
  const table = schema.tables.find((t) => t.name === tableName);
  if (!table) {
    yield* Effect.fail(new Error(`Table ${tableName} not found in schema`));
    return;
  }

  const decoded = { ...record } as any;
  for (const field of Object.keys(table.columns)) {
    const column = table.columns[field] as any;
    if (column.recordId && decoded[field] != null) {
      const [table, ...idParts] = decoded[field].split(":");
      decoded[field] = new RecordId(table, idParts.join(":"));
    }
  }

  return decoded;
});

export const generateNewId = Effect.fn("generateNewId")(function* <
  S extends SchemaStructure,
  T extends TableNames<S>
>(tableName: T) {
  return new RecordId(tableName, Uuid.v4().toString().replace(/-/g, ""));
});
