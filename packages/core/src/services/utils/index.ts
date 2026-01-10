import { GetTable, SchemaStructure, TableModel, TableNames } from '@spooky/query-builder';
import { Uuid, RecordId } from 'surrealdb';

export const parseRecordIdString = (id: string): RecordId<string> => {
  const [table, ...idParts] = id.split(':');
  return new RecordId(table, idParts.join(':'));
};

import { createLogger } from '../logger/index.js';

const logger = createLogger('info').child({ module: 'utils' });

export function decodeFromSpooky<S extends SchemaStructure, T extends TableNames<S>>(
  schema: S,
  tableName: T,
  record: TableModel<GetTable<S, T>>
): TableModel<GetTable<S, T>> {
  const table = schema.tables.find((t) => t.name === tableName);
  if (!table) {
    throw new Error(`Table ${tableName} not found in schema`);
  }

  const encoded = { ...record } as any;

  for (const field of Object.keys(table.columns)) {
    const column = table.columns[field] as any;
    const relation = schema.relationships.find((r) => r.from === tableName && r.field === field);
    if ((column.recordId || relation) && encoded[field] != null) {
      if (encoded[field] instanceof RecordId) {
        encoded[field] = `${encoded[field].table.toString()}:${encoded[field].id}`;
      } else if (
        relation &&
        (encoded[field] instanceof Object || encoded[field] instanceof Array)
      ) {
        if (Array.isArray(encoded[field])) {
          encoded[field] = encoded[field].map((item) =>
            decodeFromSpooky(schema, relation.to, item)
          );
        } else {
          encoded[field] = decodeFromSpooky(schema, relation.to, encoded[field]);
        }
      }
    }
  }

  logger.trace({ encoded }, 'Decoded record');

  return encoded as TableModel<GetTable<S, T>>;
}

export function encodeToSpooky<
  S extends SchemaStructure,
  T extends TableNames<S>,
  R extends Partial<TableModel<GetTable<S, T>>>,
>(schema: S, tableName: T, record: R): R {
  const table = schema.tables.find((t) => t.name === tableName);
  if (!table) {
    throw new Error(`Table ${tableName} not found in schema`);
  }

  const decoded = { ...record } as any;
  for (const field of Object.keys(table.columns)) {
    const column = table.columns[field] as any;

    if (column.recordId && decoded[field] != null) {
      if (decoded[field] instanceof RecordId) {
        decoded[field] = decoded[field];
      } else if (typeof decoded[field] === 'string') {
        if (!decoded[field].includes(':')) {
          decoded[field] = new RecordId(tableName, decoded[field]);
        } else {
          decoded[field] = parseRecordIdString(decoded[field]);
        }
      }
    }

    if (column.dateTime && decoded[field] != null) {
      if (typeof decoded[field] === 'string') {
        decoded[field] = new Date(decoded[field]);
      } else if (decoded[field] instanceof Date) {
        decoded[field] = decoded[field];
      }
    }
  }

  return decoded;
}

export function generateNewId<S extends SchemaStructure, T extends TableNames<S>>(
  tableName: T
): RecordId {
  return new RecordId(tableName, Uuid.v4().toString().replace(/-/g, ''));
}
