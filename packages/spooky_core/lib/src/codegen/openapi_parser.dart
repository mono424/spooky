import 'package:yaml/yaml.dart';

/// A typed argument of a backend route (from an OpenAPI requestBody schema).
class RouteArg {
  RouteArg({required this.name, required this.type, required this.optional});

  /// JSON-schema type mapped to a Dart type: string/integer/number/boolean ->
  /// String/int/num/bool; anything else -> dynamic.
  final String name;
  final String type;
  final bool optional;
}

/// A single backend route (e.g. POST `/spookify`).
class RouteDef {
  RouteDef({required this.path, required this.args});
  final String path;
  final List<RouteArg> args;
}

/// A backend: its name (the key used in `run(name, ...)`) and its routes.
class BackendDef {
  BackendDef({required this.name, this.outboxTable, required this.routes});
  final String name;
  final String? outboxTable;
  final List<RouteDef> routes;
}

/// Parse an OpenAPI document into a [BackendDef] for backend [name].
///
/// Reads each `paths.<route>.post.requestBody.content.application/json.schema`,
/// taking `properties` (with `type`) and `required`. Only the post body is
/// considered (Spooky backend routes are POST). Unsupported arg types fall
/// back to `dynamic`.
BackendDef parseOpenApi(String name, String yamlSource, {String? outboxTable}) {
  final doc = loadYaml(yamlSource);
  final paths = (doc is YamlMap ? doc['paths'] : null);
  final routes = <RouteDef>[];

  if (paths is YamlMap) {
    for (final entry in paths.entries) {
      final path = entry.key.toString();
      final op = entry.value;
      final post = op is YamlMap ? op['post'] : null;
      if (post is! YamlMap) continue;

      final schema = _dig(post, [
        'requestBody',
        'content',
        'application/json',
        'schema',
      ]);
      routes.add(RouteDef(path: path, args: _argsFromSchema(schema)));
    }
  }

  // Stable order so codegen output is deterministic.
  routes.sort((a, b) => a.path.compareTo(b.path));
  return BackendDef(name: name, outboxTable: outboxTable, routes: routes);
}

List<RouteArg> _argsFromSchema(Object? schema) {
  if (schema is! YamlMap) return const [];
  final props = schema['properties'];
  if (props is! YamlMap) return const [];
  final required = <String>{
    if (schema['required'] is YamlList)
      ...(schema['required'] as YamlList).map((e) => e.toString()),
  };

  final args = <RouteArg>[];
  for (final entry in props.entries) {
    final name = entry.key.toString();
    final spec = entry.value;
    final jsonType = spec is YamlMap ? spec['type']?.toString() : null;
    args.add(RouteArg(
      name: name,
      type: _dartType(jsonType),
      optional: !required.contains(name),
    ));
  }
  args.sort((a, b) => a.name.compareTo(b.name));
  return args;
}

String _dartType(String? jsonType) {
  switch (jsonType) {
    case 'string':
      return 'String';
    case 'integer':
      return 'int';
    case 'number':
      return 'num';
    case 'boolean':
      return 'bool';
    default:
      return 'dynamic';
  }
}

Object? _dig(YamlMap root, List<String> path) {
  Object? node = root;
  for (final key in path) {
    if (node is YamlMap && node.containsKey(key)) {
      node = node[key];
    } else {
      return null;
    }
  }
  return node;
}
