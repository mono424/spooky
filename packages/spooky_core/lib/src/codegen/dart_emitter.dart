import 'schema_parser.dart';

/// Emit the `ColumnSchema` map literal consumed by `Sp00kyConfig.schema`.
String emitSchemaMap(List<TableDef> tables) {
  final buf = StringBuffer('final spookySchema = <String, dynamic>{\n');
  for (final table in tables) {
    buf.writeln("  '${table.name}': {");
    buf.writeln('    \'columns\': {');
    for (final field in table.fields) {
      buf.writeln("      '${field.name}': ${_columnSchema(field)},");
    }
    buf.writeln('    },');
    buf.writeln('  },');
  }
  buf.writeln('};');
  return buf.toString();
}

String _columnSchema(FieldDef f) {
  final args = <String>["type: '${f.type}'"];
  if (f.isRecord) args.add('recordId: true');
  if (f.isDateTime) args.add('dateTime: true');
  if (f.optional) args.add('optional: true');
  return 'ColumnSchema(${args.join(', ')})';
}

/// Emit a typed model class per table (fields kept as declared; record ids and
/// datetimes mapped to `String` / `DateTime`).
String emitModels(List<TableDef> tables) {
  final buf = StringBuffer();
  for (final table in tables) {
    final className = _pascal(table.name);
    final fields = [
      FieldDef(
          name: 'id',
          type: 'string',
          optional: false,
          isRecord: false,
          isDateTime: false),
      ...table.fields.where((f) => f.name != 'id'),
    ];

    buf.writeln('class $className {');
    // Constructor.
    buf.writeln('  $className({');
    for (final f in fields) {
      final req = f.optional ? '' : 'required ';
      buf.writeln('    ${req}this.${f.name},');
    }
    buf.writeln('  });');
    buf.writeln();
    // Fields.
    for (final f in fields) {
      buf.writeln('  final ${_dartType(f)} ${f.name};');
    }
    buf.writeln();
    // fromJson.
    buf.writeln('  factory $className.fromJson(Map<String, dynamic> json) =>');
    buf.writeln('      $className(');
    for (final f in fields) {
      buf.writeln('        ${f.name}: ${_fromJson(f)},');
    }
    buf.writeln('      );');
    buf.writeln();
    // toJson.
    buf.writeln('  Map<String, dynamic> toJson() => {');
    for (final f in fields) {
      buf.writeln("        '${f.name}': ${_toJson(f)},");
    }
    buf.writeln('      };');
    buf.writeln('}');
    buf.writeln();
  }
  return buf.toString().trimRight();
}

String _dartType(FieldDef f) {
  final base = _baseDartType(f);
  return f.optional ? '$base?' : base;
}

String _baseDartType(FieldDef f) {
  if (f.isRecord) return 'String'; // record id encoded as `table:id`
  if (f.isDateTime) return 'DateTime';
  final t = f.type.toLowerCase();
  if (t == 'string' || t == 'uuid') return 'String';
  if (t == 'int') return 'int';
  if (t == 'number' || t == 'float' || t == 'decimal') return 'num';
  if (t == 'bool') return 'bool';
  if (t == 'object') return 'Map<String, dynamic>';
  final arr = RegExp(r'^array\s*<\s*(.*)\s*>$').firstMatch(t);
  if (arr != null) {
    final inner = _baseDartTypeForName(arr.group(1)!.trim());
    return 'List<$inner>';
  }
  if (t == 'array') return 'List<dynamic>';
  return 'dynamic';
}

String _baseDartTypeForName(String t) {
  if (t == 'string' || t == 'uuid' || t.startsWith('record')) return 'String';
  if (t == 'int') return 'int';
  if (t == 'number' || t == 'float' || t == 'decimal') return 'num';
  if (t == 'bool') return 'bool';
  if (t == 'datetime') return 'DateTime';
  return 'dynamic';
}

String _fromJson(FieldDef f) {
  final base = _baseDartType(f);
  final access = "json['${f.name}']";
  if (f.isDateTime) {
    return f.optional
        ? '$access == null ? null : DateTime.parse($access as String)'
        : 'DateTime.parse($access as String)';
  }
  if (base.startsWith('List<')) {
    final inner = base.substring(5, base.length - 1);
    return f.optional
        ? '($access as List?)?.cast<$inner>()'
        : '($access as List).cast<$inner>()';
  }
  if (base == 'dynamic') return access;
  return f.optional ? '$access as $base?' : '$access as $base';
}

String _toJson(FieldDef f) {
  if (f.isDateTime) {
    return f.optional
        ? '${f.name}?.toUtc().toIso8601String()'
        : '${f.name}.toUtc().toIso8601String()';
  }
  return f.name;
}

/// PascalCase a (possibly snake_case) table name: `blog_post` -> `BlogPost`.
String _pascal(String name) => name
    .split(RegExp(r'[_\s]+'))
    .where((p) => p.isNotEmpty)
    .map((p) => p[0].toUpperCase() + p.substring(1))
    .join();

/// Emit a full, analyzable Dart source file: imports + schema map + models.
String generateDartSource(String surql) {
  final tables = parseSchema(surql);
  final buf = StringBuffer();
  buf.writeln('// GENERATED CODE - DO NOT EDIT BY HAND.');
  buf.writeln('// Generated from a SurrealQL schema by spooky_core codegen.');
  buf.writeln();
  buf.writeln("import 'package:spooky_core/spooky_core.dart';");
  buf.writeln();
  buf.writeln(emitSchemaMap(tables));
  buf.writeln();
  buf.writeln(emitModels(tables));
  buf.writeln();
  return buf.toString();
}
