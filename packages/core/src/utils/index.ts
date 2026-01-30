import { GetTable, SchemaStructure, TableModel, TableNames } from '@spooky/query-builder';
import { Uuid, RecordId, Duration } from 'surrealdb';
import { Logger } from '../services/logger/index.js';
import { QueryTimeToLive } from '../types.js';

export * from './surql.js';
export * from './parser.js';

// ==================== RECORDID UTILITIES ====================

export const compareRecordIds = (
  a: RecordId<string> | string,
  b: RecordId<string> | string
): boolean => {
  const nA = a instanceof RecordId ? encodeRecordId(a) : a;
  const nB = b instanceof RecordId ? encodeRecordId(b) : b;
  return nA === nB;
};

export const encodeRecordId = (recordId: RecordId<string>): string => {
  return `${recordId.table.toString()}:${recordId.id}`;
};

export const extractIdPart = (id: string | RecordId<string>): string => {
  if (typeof id === 'string') {
    return id.split(':').slice(1).join(':');
  }
  // RecordId.id can be string, number, object, or array
  const idValue = id.id;
  if (typeof idValue === 'string') {
    return idValue;
  }
  // For other types (number, object, array), convert to string
  return String(idValue);
};

export const extractTablePart = (id: string | RecordId<string>): string => {
  if (typeof id === 'string') {
    return id.split(':')[0];
  }
  return id.table.toString();
};

export const parseRecordIdString = (id: string): RecordId<string> => {
  const [table, ...idParts] = id.split(':');
  return new RecordId(table, idParts.join(':'));
};

export function generateId(): string {
  return Uuid.v4().toString().replace(/-/g, '');
}

export function generateNewTableId<S extends SchemaStructure, T extends TableNames<S>>(
  tableName: T
): RecordId {
  return new RecordId(tableName, generateId());
}

// ==================== SCHEMA ENCODING/DECODING ====================

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

  return encoded as TableModel<GetTable<S, T>>;
}

// ==================== TIME/DURATION UTILITIES ====================

/**
 * Parse duration string or Duration object to milliseconds
 */
export function parseDuration(duration: QueryTimeToLive | Duration): number {
  if (duration instanceof Duration) {
    const ms = (duration as any).milliseconds || (duration as any)._milliseconds;
    if (ms) return Number(ms);
    const str = duration.toString();
    if (str !== '[object Object]') return parseDuration(str as any);
    return 600000;
  }

  if (typeof duration === 'bigint') {
    return Number(duration);
  }

  if (typeof duration !== 'string') return 600000;

  const match = duration.match(/^(\d+)([smh])$/);
  if (!match) return 600000;
  const val = parseInt(match[1], 10);
  const unit = match[2];
  switch (unit) {
    case 's':
      return val * 1000;
    case 'h':
      return val * 3600000;
    case 'm':
    default:
      return val * 60000;
  }
}

// ==================== DATABASE UTILITIES ====================

/**
 * Helper for retrying DB operations with exponential backoff
 */
export async function withRetry<T>(
  logger: Logger,
  operation: () => Promise<T>,
  retries = 3,
  delayMs = 100
): Promise<T> {
  let lastError;
  for (let i = 0; i < retries; i++) {
    try {
      return await operation();
    } catch (err: any) {
      lastError = err;
      if (
        err?.message?.includes('Can not open transaction') ||
        err?.message?.includes('transaction') ||
        err?.message?.includes('Database is busy')
      ) {
        const msg = err instanceof Error ? err.message : String(err);
        logger.warn(
          {
            attempt: i + 1,
            retries,
            error: msg,
            Category: 'spooky-client::utils::withRetry',
          },
          'Retrying DB operation'
        );
        await new Promise((res) => setTimeout(res, delayMs * (i + 1)));
        continue;
      }
      throw err;
    }
  }
  throw lastError;
}
