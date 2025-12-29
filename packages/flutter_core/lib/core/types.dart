enum QueryTimeToLive {
  oneMinute,
  fiveMinutes,
  tenMinutes,
  fifteenMinutes,
  twentyMinutes,
  twentyFiveMinutes,
  thirtyMinutes,
  oneHour,
  twoHours,
  threeHours,
  fourHours,
  fiveHours,
  sixHours,
  sevenHours,
  eightHours,
  nineHours,
  tenHours,
  elevenHours,
  twelveHours,
  oneDay,
}

/// Extension, um das Enum in den String-Wert für SurrealDB umzuwandeln
/// und von einem String zurück in das Enum zu parsen.
extension QueryTimeToLiveExtension on QueryTimeToLive {
  /// Gibt den String-Wert zurück (z.B. '10m', '1h').
  String get value {
    switch (this) {
      case QueryTimeToLive.oneMinute:
        return '1m';
      case QueryTimeToLive.fiveMinutes:
        return '5m';
      case QueryTimeToLive.tenMinutes:
        return '10m';
      case QueryTimeToLive.fifteenMinutes:
        return '15m';
      case QueryTimeToLive.twentyMinutes:
        return '20m';
      case QueryTimeToLive.twentyFiveMinutes:
        return '25m';
      case QueryTimeToLive.thirtyMinutes:
        return '30m';
      case QueryTimeToLive.oneHour:
        return '1h';
      case QueryTimeToLive.twoHours:
        return '2h';
      case QueryTimeToLive.threeHours:
        return '3h';
      case QueryTimeToLive.fourHours:
        return '4h';
      case QueryTimeToLive.fiveHours:
        return '5h';
      case QueryTimeToLive.sixHours:
        return '6h';
      case QueryTimeToLive.sevenHours:
        return '7h';
      case QueryTimeToLive.eightHours:
        return '8h';
      case QueryTimeToLive.nineHours:
        return '9h';
      case QueryTimeToLive.tenHours:
        return '10h';
      case QueryTimeToLive.elevenHours:
        return '11h';
      case QueryTimeToLive.twelveHours:
        return '12h';
      case QueryTimeToLive.oneDay:
        return '1d';
    }
  }

  /// Erstellt ein Enum aus einem String (z.B. aus der DB).
  /// Fallback ist '10m', falls der String unbekannt ist (analog zur TS-Logik).
  static QueryTimeToLive fromString(String val) {
    try {
      return QueryTimeToLive.values.firstWhere((e) => e.value == val);
    } catch (_) {
      // In query/incantation.ts ist der Default 600000ms (= 10m)
      return QueryTimeToLive.tenMinutes;
    }
  }
}

class DatabaseConfig {
  final String? endpoint;
  final String path;
  final String namespace;
  final String database;
  final String? token;
  final String? devUrl;
  final int? devSidecarPort;

  DatabaseConfig({
    this.endpoint,
    required this.path,
    required this.namespace,
    required this.database,
    this.token,
    this.devUrl,
    this.devSidecarPort,
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

  factory RecordId.fromString(String rawId) {
    final [table, id] = rawId.split(':');
    return RecordId(table, id);
  }
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
