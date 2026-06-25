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

// `@nosync` table marker. Source schemas carry it as a `-- @nosync` comment
// line preceding the DEFINE TABLE; once the CLI materializes it, the server
// DEFINE TABLE carries a `COMMENT 'sp00ky:nosync'` clause instead.
final _nosyncComment = RegExp(r'--\s*@nosync\b', caseSensitive: false);
const _nosyncTableComment = 'sp00ky:nosync';

final _defineField = RegExp(
    r'^DEFINE\s+FIELD\s+(?:IF\s+NOT\s+EXISTS\s+|OVERWRITE\s+)?([A-Za-z_][A-Za-z0-9_]*)\s+ON\s+(?:TABLE\s+)?([A-Za-z_][A-Za-z0-9_]*)\s+TYPE\s+(.*)$',
    caseSensitive: false);

// Clause keywords that can follow the TYPE expression in a DEFINE FIELD.
final _typeTerminator = RegExp(
    r'\s+(ASSERT|DEFAULT|VALUE|PERMISSIONS|READONLY|COMMENT|REFERENCE|FLEXIBLE)\b',
    caseSensitive: false);

/// A `DEFINE ACCESS` definition's signup/signin variable names (the `$vars`
/// referenced in its SIGNUP/SIGNIN bodies). SurrealQL declares no types for
/// these, so the generator emits them as `String`.
class AccessDef {
  AccessDef({
    required this.name,
    required this.signupParams,
    required this.signinParams,
  });
  final String name;
  final List<String> signupParams;
  final List<String> signinParams;
}

/// Tables + access methods parsed from a schema (backends come from OpenAPI).
class ParsedSchema {
  ParsedSchema({required this.tables, required this.accesses});
  final List<TableDef> tables;
  final List<AccessDef> accesses;
}

/// Parse tables and `DEFINE ACCESS` methods from a schema.
ParsedSchema parseProject(String surql) =>
    ParsedSchema(tables: parseSchema(surql), accesses: parseAccesses(surql));

// SurrealQL built-in vars that are never user-supplied signup/signin params.
const _builtinVars = {
  'auth',
  'value',
  'access',
  'token',
  'session',
  'scope',
  'before',
  'after',
  'this',
  'parent',
  'input',
  'event',
};

final _defineAccess =
    RegExp(r'DEFINE\s+ACCESS\s+([A-Za-z_][A-Za-z0-9_]*)', caseSensitive: false);

/// Extract [AccessDef]s by scanning the full source (access blocks span
/// braces/parens with internal `;`, so they survive the statement splitter).
List<AccessDef> parseAccesses(String surql) {
  final src = _stripComments(surql);
  final out = <AccessDef>[];
  for (final m in _defineAccess.allMatches(src)) {
    final name = m.group(1)!;
    final rest = src.substring(m.end);
    out.add(AccessDef(
      name: name,
      signupParams: _varsInSection(rest, 'SIGNUP'),
      signinParams: _varsInSection(rest, 'SIGNIN'),
    ));
  }
  return out;
}

/// `$var` names referenced inside the balanced `{...}`/`(...)` body following
/// [keyword], minus SurrealQL built-ins, de-duplicated in first-seen order.
List<String> _varsInSection(String block, String keyword) {
  final kw = RegExp('\\b$keyword\\b', caseSensitive: false).firstMatch(block);
  if (kw == null) return const [];
  final body = _balancedSpan(block, kw.end);
  if (body == null) return const [];
  final seen = <String>{};
  final result = <String>[];
  for (final v in RegExp(r'\$([A-Za-z_][A-Za-z0-9_]*)').allMatches(body)) {
    final name = v.group(1)!;
    if (_builtinVars.contains(name)) continue;
    if (seen.add(name)) result.add(name);
  }
  return result;
}

/// Starting at [from], skip whitespace to the first `{` or `(`, then return the
/// substring up to its matching close (handling nesting). Null if none.
String? _balancedSpan(String s, int from) {
  var i = from;
  while (i < s.length && s[i].trim().isEmpty) {
    i++;
  }
  if (i >= s.length) return null;
  final open = s[i];
  final close = open == '{' ? '}' : (open == '(' ? ')' : null);
  if (close == null) return null;
  var depth = 0;
  final start = i;
  for (; i < s.length; i++) {
    if (s[i] == open) depth++;
    if (s[i] == close) {
      depth--;
      if (depth == 0) return s.substring(start + 1, i);
    }
  }
  return null;
}

/// Parse a SurrealQL schema into ordered [TableDef]s. Tables are emitted in the
/// order their `DEFINE TABLE` appears; fields in declaration order. Statements
/// other than DEFINE TABLE/FIELD are ignored.
List<TableDef> parseSchema(String surql) {
  final tables = <String, TableDef>{};
  final order = <String>[];
  // Tables marked `@nosync` are server-only and excluded from the generated
  // client schema (mirrors the TS CLI, which strips them too). `_00_`-prefixed
  // system/meta tables (e.g. `_00_user_feature`) are excluded the same way:
  // they are synced and read at runtime but must never appear in generated
  // types. Their fields are skipped so a trailing DEFINE FIELD can't resurrect
  // the table.
  final nosync = <String>{};

  TableDef tableFor(String name) {
    final existing = tables[name];
    if (existing != null) return existing;
    final t = TableDef(name);
    tables[name] = t;
    order.add(name);
    return t;
  }

  // Collect `-- @nosync` tables from the original text first: that marker is a
  // comment line preceding the DEFINE TABLE, so it must be read before comments
  // are stripped. (The materialized `COMMENT 'sp00ky:nosync'` form survives
  // stripping and is handled in the main loop below.)
  nosync.addAll(_nosyncTablesFromComments(surql));

  // Strip line comments BEFORE splitting on `;`. A `;` inside a `--` comment
  // (e.g. "owner of this broadcast (the publisher; = relay stream id)") would
  // otherwise split mid-statement, leaving the following `DEFINE FIELD` with a
  // junk prefix so its `^`-anchored regex never matches — silently dropping the
  // field (this is exactly how `broadcast.owner`/`restream_providers` vanished).
  final clean = _stripComments(surql);

  for (final raw in clean.split(';')) {
    final stmt = raw.trim();
    if (stmt.isEmpty) continue;

    final tableMatch = _defineTable.firstMatch(stmt);
    if (tableMatch != null) {
      final name = tableMatch.group(1)!;
      if (name.startsWith('_00_') ||
          nosync.contains(name) ||
          stmt.contains(_nosyncTableComment)) {
        nosync.add(name);
        continue;
      }
      tableFor(name);
      continue;
    }

    final fieldMatch = _defineField.firstMatch(stmt);
    if (fieldMatch != null) {
      final fieldName = fieldMatch.group(1)!;
      final tableName = fieldMatch.group(2)!;
      if (nosync.contains(tableName)) continue;
      final typeExpr = _extractType(fieldMatch.group(3)!);
      tableFor(tableName).fields.add(_buildField(fieldName, typeExpr));
    }
  }

  return [for (final name in order) tables[name]!];
}

/// Tables marked with a `-- @nosync` comment line immediately preceding their
/// `DEFINE TABLE`. Scanned on the raw schema (before comment stripping).
Set<String> _nosyncTablesFromComments(String surql) {
  final result = <String>{};
  var pending = false;
  for (final line in surql.split('\n')) {
    if (_nosyncComment.hasMatch(line)) {
      pending = true;
      continue;
    }
    final m = _defineTable.firstMatch(line.trimLeft());
    if (m != null) {
      if (pending) result.add(m.group(1)!);
      pending = false;
    }
  }
  return result;
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
