import {
  GetTable,
  RecordId,
  SchemaStructure,
  TableModel,
  TableNames,
} from "@spooky/query-builder";
import { Effect } from "effect";

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
      if (encoded[field] instanceof RecordId) {
        encoded[field] = (encoded[field] as RecordId).toString();
      } else if (Array.isArray(encoded[field])) {
        encoded[field] =
          (encoded[field] as unknown[])?.map((item) =>
            item instanceof RecordId ? item.toString() : item
          ) ?? [];
      }
    }
  }

  return encoded as TableModel<GetTable<S, T>>;
});

export const encodeToSpooky = Effect.fn("encodeToSpooky")(function* <
  S extends SchemaStructure,
  T extends TableNames<S>
>(schema: S, tableName: T, record: TableModel<GetTable<S, T>>) {
  const table = schema.tables.find((t) => t.name === tableName);
  if (!table) {
    yield* Effect.fail(new Error(`Table ${tableName} not found in schema`));
    return;
  }

  const decoded = { ...record } as any;

  for (const field of Object.keys(table.columns)) {
    const column = table.columns[field] as any;
    if (column.recordId && decoded[field] != null) {
      // Find the relationship to get the related table name
      const relationship = schema.relationships.find(
        (r) => r.from === tableName && r.field === field
      );

      if (relationship) {
        const relatedTable = relationship.to;

        if (Array.isArray(decoded[field])) {
          // Handle many relationships (arrays)
          decoded[field] =
            (decoded[field] as string[])?.map((id) => {
              if (typeof id === "string") {
                if (id.includes(":")) {
                  const [table, ...idParts] = id.split(":");
                  return new RecordId(table, idParts.join(":"));
                }
                return new RecordId(relatedTable, id);
              }
              return id;
            }) ?? [];
        } else if (typeof decoded[field] === "string") {
          // Handle one relationships (single values)
          const id = decoded[field] as string;
          if (id.includes(":")) {
            const [table, ...idParts] = id.split(":");
            decoded[field] = new RecordId(table, idParts.join(":"));
          } else {
            decoded[field] = new RecordId(relatedTable, id);
          }
        }
      } else if (field === "id" && typeof decoded[field] === "string") {
        // Handle the id field itself
        const id = decoded[field] as string;
        if (id.includes(":")) {
          const [table, ...idParts] = id.split(":");
          decoded[field] = new RecordId(table, idParts.join(":"));
        } else {
          decoded[field] = new RecordId(tableName, id);
        }
      }
    }
  }

  yield* Effect.succeed(decoded as TableModel<GetTable<S, T>>);
});
