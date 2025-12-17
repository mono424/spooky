class SpookyConfig {
  final String schemaString;
  final String globalDBurl;
  final String localDBurl;
  final String dbName;
  final String namespace;
  final String database;
  final String internalDatabase; // Used for Spooky's internal state
  final String? token; // Added token for auth if needed

  const SpookyConfig({
    required this.schemaString,
    required this.globalDBurl,
    required this.localDBurl,
    required this.dbName,
    required this.namespace,
    required this.database,
    required this.internalDatabase,
    this.token,
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
