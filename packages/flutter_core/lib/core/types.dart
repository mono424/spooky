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

class RecordId {
  final String table;
  final dynamic id;

  const RecordId(this.table, this.id);

  // Verhält sich wie in TS/Rust, wenn man es ausgibt
  @override
  String toString() => '$table:$id';

  // Hilfreich für Vergleiche
  @override
  bool operator ==(Object other) =>
      other is RecordId && other.table == table && other.id == id;

  @override
  int get hashCode => table.hashCode ^ id.hashCode;
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
