/// Extracts per-table `PERMISSIONS FOR select` predicates from a schema's
/// SURQL so the client circuit can be seeded (the SSP server does the same at
/// boot). Faithful port of `apps/ssp/src/lib.rs::extract_select_permission_text`
/// plus a `DEFINE TABLE` statement splitter (the server gets table defs from a
/// live `INFO FOR DB` query; the client parses them out of `schemaSurql`).

/// Parse `{ table: selectWhereText }` from the schema SURQL.
///
/// The returned text is what `Circuit::set_permission` expects: a raw WHERE
/// body, or `'true'` (full / no clause) / `'false'` (none / deny).
Map<String, String> extractTablePermissions(String schemaSurql) {
  final out = <String, String>{};
  for (final def in _defineTableStatements(schemaSurql)) {
    final name = _tableName(def);
    if (name == null) continue;
    out[name] = extractSelectPermissionText(def);
  }
  return out;
}

/// Split a schema script into individual `DEFINE TABLE ...` statements
/// (terminated by `;`). Other statement kinds are ignored.
Iterable<String> _defineTableStatements(String schemaSurql) sync* {
  // Statements are `;`-terminated. Naive split is sufficient because table
  // permission WHERE bodies in these schemas do not contain literal
  // semicolons (SurrealDB statement terminators).
  //
  // Strip full-line `--` comments FIRST: comment lines aren't `;`-terminated,
  // so the `-- SECTION` headers that precede each `DEFINE TABLE` would
  // otherwise land at the front of the split chunk (`-- GAME TABLE\n…\nDEFINE
  // TABLE game …`), defeating the `startsWith('DEFINE TABLE')` check and
  // leaving every table default-deny.
  for (final raw in _stripLineComments(schemaSurql).split(';')) {
    final stmt = raw.trim();
    if (stmt.isEmpty) continue;
    final upper = stmt.toUpperCase();
    // Accept "DEFINE TABLE [IF NOT EXISTS|OVERWRITE] <name>".
    if (upper.startsWith('DEFINE TABLE')) {
      yield stmt;
    }
  }
}

/// Drop whole-line SurrealQL comments (a line whose first non-whitespace is
/// `--`). Leaves inline content untouched; the schemas this parses keep their
/// comments on their own lines.
String _stripLineComments(String schemaSurql) {
  final out = StringBuffer();
  for (final line in schemaSurql.split('\n')) {
    if (line.trimLeft().startsWith('--')) continue;
    out.writeln(line);
  }
  return out.toString();
}

/// Pull the table name out of a `DEFINE TABLE [IF NOT EXISTS] <name> ...` def.
String? _tableName(String defineTable) {
  final tokens = defineTable.trim().split(RegExp(r'\s+'));
  // tokens[0]=DEFINE tokens[1]=TABLE, then optional modifiers, then the name.
  var i = 2;
  // Skip IF NOT EXISTS / OVERWRITE.
  while (i < tokens.length) {
    final t = tokens[i].toUpperCase();
    if (t == 'IF' || t == 'NOT' || t == 'EXISTS' || t == 'OVERWRITE') {
      i++;
    } else {
      break;
    }
  }
  if (i >= tokens.length) return null;
  return tokens[i];
}

/// Faithful port of `extract_select_permission_text`.
String extractSelectPermissionText(String defineTable) {
  final def = defineTable.trim().replaceAll(RegExp(r';+$'), '');
  final upper = def.toUpperCase();

  final permIdx = upper.indexOf('PERMISSIONS');
  if (permIdx < 0) {
    // No PERMISSIONS clause -> SurrealDB defaults to FULL -> allow.
    return 'true';
  }
  final permSection = def.substring(permIdx + 'PERMISSIONS'.length).trim();
  final permUpper = permSection.toUpperCase();

  if (permUpper.startsWith('FULL')) return 'true';
  if (permUpper.startsWith('NONE')) return 'false';

  final lower = permSection.toLowerCase();
  final clauseStarts = <int>[];
  for (final m in RegExp('for ').allMatches(lower)) {
    final i = m.start;
    if (i == 0 || _isAsciiWhitespace(lower.codeUnitAt(i - 1))) {
      clauseStarts.add(i);
    }
  }
  if (clauseStarts.isEmpty) {
    // PERMISSIONS clause has no FOR clauses; deny.
    return 'false';
  }

  for (var idx = 0; idx < clauseStarts.length; idx++) {
    final start = clauseStarts[idx];
    final end = idx + 1 < clauseStarts.length
        ? clauseStarts[idx + 1]
        : permSection.length;
    final clause = permSection.substring(start, end);
    final lowerClause = clause.toLowerCase();
    final whereIdx = lowerClause.indexOf('where');
    final header = whereIdx >= 0 ? clause.substring(0, whereIdx) : clause;
    if (!header.toLowerCase().contains('select')) {
      continue;
    }
    if (whereIdx < 0) {
      // FOR select with no WHERE -> full-allow.
      return 'true';
    }
    final body = clause
        .substring(whereIdx + 'where'.length)
        .trim()
        .replaceAll(RegExp(r'[,;]+$'), '')
        .trim();
    if (body.isEmpty) return 'true';
    return body;
  }
  // No select clause found among the FOR clauses -> deny.
  return 'false';
}

bool _isAsciiWhitespace(int c) =>
    c == 0x20 || c == 0x09 || c == 0x0a || c == 0x0d || c == 0x0c || c == 0x0b;
