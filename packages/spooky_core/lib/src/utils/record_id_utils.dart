import '../surreal/value.dart';

/// Encode a [RecordId] to `table:id` (TS `encodeRecordId`).
String encodeRecordId(RecordId recordId) => '${recordId.table}:${recordId.id}';

/// Structural equality of two ids, accepting either [RecordId] or [String]
/// (TS `compareRecordIds`).
bool compareRecordIds(Object a, Object b) {
  final nA = a is RecordId ? encodeRecordId(a) : a as String;
  final nB = b is RecordId ? encodeRecordId(b) : b as String;
  return nA == nB;
}

/// The id part of a `table:id` value (TS `extractIdPart`).
String extractIdPart(Object id) {
  if (id is String) {
    return id.split(':').skip(1).join(':');
  }
  final recordId = id as RecordId;
  return recordId.id.toString();
}

/// The table part of a `table:id` value (TS `extractTablePart`).
String extractTablePart(Object id) {
  if (id is String) {
    return id.split(':').first;
  }
  return (id as RecordId).table;
}

/// Parse a `table:id` string into a [RecordId] (TS `parseRecordIdString`).
RecordId parseRecordIdString(String id) => RecordId.parse(id);
