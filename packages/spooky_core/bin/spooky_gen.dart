import 'dart:io';

import 'package:spooky_core/codegen.dart';

/// CLI: generate Dart models + schema map from a SurrealQL schema file.
///
/// Usage: dart run spooky_core:spooky_gen <schema.surql> [output.dart]
/// Writes to stdout when no output path is given.
void main(List<String> args) {
  if (args.isEmpty) {
    stderr.writeln(
        'Usage: dart run spooky_core:spooky_gen <schema.surql> [output.dart]');
    exitCode = 64; // EX_USAGE
    return;
  }

  final input = File(args[0]);
  if (!input.existsSync()) {
    stderr.writeln('Schema file not found: ${args[0]}');
    exitCode = 66; // EX_NOINPUT
    return;
  }

  final source = generateDartSource(input.readAsStringSync());

  if (args.length > 1) {
    File(args[1]).writeAsStringSync(source);
    stdout.writeln('Wrote ${args[1]}');
  } else {
    stdout.write(source);
  }
}
