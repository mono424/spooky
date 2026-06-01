import '../surreal/value.dart';
import 'record_id_utils.dart';

/// Minimal column schema shape the parser needs (subset of the TS
/// `ColumnSchema` from query-builder).
class ColumnSchema {
  const ColumnSchema({
    this.recordId = false,
    this.dateTime = false,
    this.type,
    this.optional = false,
  });

  final bool recordId;
  final bool dateTime;
  final String? type;
  final bool optional;
}

/// Strip server-only fields: keep `id`, any `_00_*` metadata, and declared
/// schema columns (TS `cleanRecord`).
Map<String, dynamic> cleanRecord(
  Map<String, ColumnSchema> tableSchema,
  Map<String, dynamic> record,
) {
  final cleaned = <String, dynamic>{};
  for (final entry in record.entries) {
    final key = entry.key;
    if (key == 'id' || key.startsWith('_00_') || tableSchema.containsKey(key)) {
      cleaned[key] = entry.value;
    }
  }
  return cleaned;
}

/// Coerce params to their schema column types (TS `parseParams`).
Map<String, dynamic> parseParams(
  Map<String, ColumnSchema> tableSchema,
  Map<String, dynamic> params,
) {
  final parsed = <String, dynamic>{};
  for (final entry in params.entries) {
    final column = tableSchema[entry.key];
    if (column != null && entry.value != null) {
      parsed[entry.key] = _parseValue(entry.key, column, entry.value);
    }
  }
  return parsed;
}

dynamic _parseValue(String name, ColumnSchema column, dynamic value) {
  if (column.recordId) {
    if (value is RecordId) return value;
    if (value is String) return parseRecordIdString(value);
    throw ArgumentError('Invalid value for $name: $value');
  }
  if (column.dateTime) {
    if (value is DateTime) return value;
    if (value is int) return DateTime.fromMillisecondsSinceEpoch(value);
    if (value is String) return DateTime.parse(value);
    throw ArgumentError('Invalid value for $name: $value');
  }
  return value;
}
