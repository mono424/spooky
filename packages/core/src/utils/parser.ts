import { ColumnSchema, RecordId } from '@spooky/query-builder';
import { parseRecordIdString } from './index.js';
import { DateTime } from 'surrealdb';

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
