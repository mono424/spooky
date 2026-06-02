/// A parsed field from a `DEFINE FIELD` statement.
class FieldDef {
  FieldDef({
    required this.name,
    required this.type,
    required this.optional,
    required this.isRecord,
    required this.isDateTime,
    this.recordTable,
  });

  /// Field name as declared (kept as-is, e.g. snake_case).
  final String name;

  /// Normalized type expression with any `option<>` stripped
  /// (e.g. `string`, `datetime`, `record`, `array<string>`).
  final String type;
  final bool optional;
  final bool isRecord;
  final bool isDateTime;

  /// For `record<x>`, the referenced table (`x`); null for bare `record`.
  final String? recordTable;
}

/// A parsed table from a `DEFINE TABLE` statement, with its fields.
class TableDef {
  TableDef(this.name) : fields = [];
  final String name;
  final List<FieldDef> fields;
}

final _defineTable = RegExp(
    r'^DEFINE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+|OVERWRITE\s+)?([A-Za-z_][A-Za-z0-9_]*)',
    caseSensitive: false);

final _defineField = RegExp(
    r'^DEFINE\s+FIELD\s+(?:IF\s+NOT\s+EXISTS\s+|OVERWRITE\s+)?([A-Za-z_][A-Za-z0-9_]*)\s+ON\s+(?:TABLE\s+)?([A-Za-z_][A-Za-z0-9_]*)\s+TYPE\s+(.*)$',
    caseSensitive: false);

// Clause keywords that can follow the TYPE expression in a DEFINE FIELD.
final _typeTerminator = RegExp(
    r'\s+(ASSERT|DEFAULT|VALUE|PERMISSIONS|READONLY|COMMENT|REFERENCE|FLEXIBLE)\b',
    caseSensitive: false);

/// Parse a SurrealQL schema into ordered [TableDef]s. Tables are emitted in the
/// order their `DEFINE TABLE` appears; fields in declaration order. Statements
/// other than DEFINE TABLE/FIELD are ignored.
List<TableDef> parseSchema(String surql) {
  final tables = <String, TableDef>{};
  final order = <String>[];

  TableDef tableFor(String name) {
    final existing = tables[name];
    if (existing != null) return existing;
    final t = TableDef(name);
    tables[name] = t;
    order.add(name);
    return t;
  }

  for (final raw in surql.split(';')) {
    final stmt = _stripComments(raw).trim();
    if (stmt.isEmpty) continue;

    final tableMatch = _defineTable.firstMatch(stmt);
    if (tableMatch != null) {
      tableFor(tableMatch.group(1)!);
      continue;
    }

    final fieldMatch = _defineField.firstMatch(stmt);
    if (fieldMatch != null) {
      final fieldName = fieldMatch.group(1)!;
      final tableName = fieldMatch.group(2)!;
      final typeExpr = _extractType(fieldMatch.group(3)!);
      tableFor(tableName).fields.add(_buildField(fieldName, typeExpr));
    }
  }

  return [for (final name in order) tables[name]!];
}

String _stripComments(String s) =>
    s.replaceAll(RegExp(r'--.*'), '').replaceAll(RegExp(r'#.*'), '');

/// Trim trailing clause keywords (ASSERT/DEFAULT/...) off the TYPE expression.
String _extractType(String afterType) {
  final term = _typeTerminator.firstMatch(afterType);
  final typePart =
      term != null ? afterType.substring(0, term.start) : afterType;
  return typePart.trim();
}

FieldDef _buildField(String name, String typeExpr) {
  var type = typeExpr.trim();
  var optional = false;

  // option<T> -> optional, inner T
  final opt =
      RegExp(r'^option\s*<\s*(.*)\s*>$', caseSensitive: false).firstMatch(type);
  if (opt != null) {
    optional = true;
    type = opt.group(1)!.trim();
  }

  final lower = type.toLowerCase();
  final isDateTime = lower == 'datetime';
  final isRecord = lower == 'record' || lower.startsWith('record<');

  String? recordTable;
  if (isRecord) {
    final rec =
        RegExp(r'^record\s*<\s*([A-Za-z_][A-Za-z0-9_]*)', caseSensitive: false)
            .firstMatch(type);
    recordTable = rec?.group(1);
    type = 'record';
  }

  return FieldDef(
    name: name,
    type: type,
    optional: optional,
    isRecord: isRecord,
    isDateTime: isDateTime,
    recordTable: recordTable,
  );
}
