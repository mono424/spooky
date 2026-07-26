/// One schema relationship, as consumed by `QueryBuilder.related`
/// (TS `SchemaStructure.relationships[n]`).
///
/// [from] declares the relationship, [field] is the name used in `.related()`
/// (and the alias in the emitted subquery), [to] is the joined table.
/// [cardinality] is `one` for a forward `record<x>` field and `many` for the
/// derived reverse direction.
class SchemaRelationship {
  const SchemaRelationship({
    required this.from,
    required this.field,
    required this.to,
    required this.cardinality,
  });

  final String from;
  final String field;
  final String to;
  final String cardinality;

  bool get isMany => cardinality == 'many';

  /// The correlation field, matching the CLI's rule: a `many` relation matches
  /// the child's field named after the PARENT table, while a `one` relation
  /// reads the parent's own field of the same name as the alias.
  String get foreignKeyField => isMany ? from : field;

  Map<String, dynamic> toMap() => {
        'from': from,
        'field': field,
        'to': to,
        'cardinality': cardinality,
      };

  static SchemaRelationship? fromMap(Object? raw) {
    if (raw is! Map) return null;
    final from = raw['from'];
    final field = raw['field'];
    final to = raw['to'];
    final cardinality = raw['cardinality'];
    if (from is! String || field is! String || to is! String) return null;
    return SchemaRelationship(
      from: from,
      field: field,
      to: to,
      cardinality: cardinality == 'many' ? 'many' : 'one',
    );
  }
}

/// Read the relationships declared in a runtime schema map (the `relationships`
/// key emitted by codegen). Returns an empty list for a schema without any, so a
/// hand-written schema keeps working — `.related()` then warn-skips.
List<SchemaRelationship> relationshipsOf(Map<String, dynamic> schema) {
  final raw = schema['relationships'];
  if (raw is! List) return const [];
  return [
    for (final entry in raw)
      if (SchemaRelationship.fromMap(entry) case final rel?) rel,
  ];
}

/// Find the relationship [field] names on [table], or null when the schema
/// declares none (an unknown relation is skipped rather than fatal, mirroring
/// the TS builder).
SchemaRelationship? findRelationship(
  List<SchemaRelationship> relationships,
  String table,
  String field,
) {
  for (final rel in relationships) {
    if (rel.from == table && rel.field == field) return rel;
  }
  return null;
}

/// Naive English pluralization for reverse-relationship field names. Must match
/// `pluralize_table_name` in `apps/cli/src/json_schema.rs` exactly, or a Dart
/// client would name its reverse relations differently from the JS client
/// generated off the same schema.
String pluralizeTableName(String table) {
  switch (table) {
    case 'user':
      return 'users';
    case 'person':
      return 'people';
    case 'child':
      return 'children';
    case 'mouse':
      return 'mice';
  }
  if (table.endsWith('s') ||
      table.endsWith('sh') ||
      table.endsWith('ch') ||
      table.endsWith('x') ||
      table.endsWith('z')) {
    return '${table}es';
  }
  if (table.endsWith('y') && table.length > 1) {
    const vowels = {'a', 'e', 'i', 'o', 'u'};
    final secondLast = table[table.length - 2];
    if (!vowels.contains(secondLast)) {
      return '${table.substring(0, table.length - 1)}ies';
    }
    return '${table}s';
  }
  if (table.endsWith('fe')) {
    return '${table.substring(0, table.length - 2)}ves';
  }
  if (table.endsWith('f')) {
    return '${table.substring(0, table.length - 1)}ves';
  }
  return '${table}s';
}
