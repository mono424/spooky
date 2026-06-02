/// SurrealQL-schema -> Dart codegen for spooky_core.
///
/// Parse a schema and emit the `ColumnSchema` map (for `Sp00kyConfig.schema`)
/// plus typed model classes. Kept out of the main `spooky_core.dart` barrel so
/// the runtime package doesn't depend on codegen.
library;

export 'src/codegen/schema_parser.dart' show parseSchema, TableDef, FieldDef;
export 'src/codegen/dart_emitter.dart'
    show emitSchemaMap, emitModels, generateDartSource;
