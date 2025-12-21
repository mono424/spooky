class DatabaseConfig {
  final String? endpoint;
  final String path;
  final String namespace;
  final String database;
  final String? token;

  DatabaseConfig({
    this.endpoint,
    required this.path,
    required this.namespace,
    required this.database,
    this.token,
  });
}

class SpookyConfig {
  final String schemaSurql;
  final String schema;
  final DatabaseConfig database;

  SpookyConfig({
    required this.schemaSurql,
    required this.schema,
    required this.database,
  });
}

// Similar to Incantation in TS
class Incantation {
  final int id;
  final String surrealql;
  int hash;
  final int lastActiveAt;

  Incantation({
    required this.id,
    required this.surrealql,
    required this.hash,
    required this.lastActiveAt,
  });

  Map<String, dynamic> toJson() => {
    'id': id,
    'surrealql': surrealql,
    'hash': hash,
    'lastActiveAt': lastActiveAt,
  };
}

typedef QueryHash = int;
