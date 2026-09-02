import type { ColumnSchema} from '@spooky-sync/query-builder';
import { RecordId, baseFieldOfParam } from '@spooky-sync/query-builder';
import { parseRecordIdString } from './index';
import { DateTime } from 'surrealdb';

export function cleanRecord(
  tableSchema: Record<string, ColumnSchema>,
  record: Record<string, any>
): Record<string, any> {
  const cleaned: Record<string, any> = {};
  for (const [key, value] of Object.entries(record)) {
    if (key === 'id' || key.startsWith('_00_') || key in tableSchema) {
      cleaned[key] = value;
    }
  }
  return cleaned;
}

/**
 * Parse a RECORD's fields against the table schema. Anything the schema does not
 * know is dropped, which is what keeps a stray field out of a write.
 */
export function parseParams(
  tableSchema: Record<string, ColumnSchema>,
  params: Record<string, any>
) {
  const parsedParams: Record<string, any> = {};
  for (const [key, value] of Object.entries(params)) {
    const column = tableSchema[key];
    if (column && value !== undefined) {
      parsedParams[key] = parseValue(key, column, value);
    }
  }

  return parsedParams;
}

/**
 * Parse a QUERY's params. Unlike a record's fields, a param name is not always a
 * column: an `_or` branch binds under a synthetic `white__or0` (see
 * `orParamName` in @spooky-sync/query-builder), and the surql that references it
 * is already written. So resolve the column through the synthetic name, and keep
 * a param we cannot type rather than dropping it.
 *
 * Dropping was the bug: `parseParams` kept only column-named params, so every
 * `_or` query registered with `$or0`/`$or1` unbound and matched no rows at all,
 * silently (rowCount 0, errorCount 0).
 */
export function parseQueryParams(
  tableSchema: Record<string, ColumnSchema>,
  params: Record<string, any>
) {
  const parsedParams: Record<string, any> = {};
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined) continue;
    const column = tableSchema[key] ?? tableSchema[baseFieldOfParam(key)];
    parsedParams[key] = column ? parseValue(key, column, value) : value;
  }

  return parsedParams;
}

function parseValue(name: string, column: ColumnSchema, value: any) {
  if (column.recordId) {
    if (value instanceof RecordId) return value;
    if (typeof value === 'string') return parseRecordIdString(value);
    throw new Error(`Invalid value for ${name}: ${value}`);
  }
  if (column.dateTime) {
    if (value instanceof Date) return value;
    if (value instanceof DateTime) return value.toDate();
    if (typeof value === 'number' || typeof value === 'string') return new Date(value);
    throw new Error(`Invalid value for ${name}: ${value}`);
  }
  return value;
}
