import 'dart:async';
import '../../types.dart'; // Importiert dein Enum und die Extension

/// Hilfsklasse für die Metadaten (in TS war das ein inline object type)
class IncantationMeta {
  final String tableName;
  IncantationMeta({required this.tableName});
}

class Incantation<T> {
  // Wir nutzen String für RecordId, bis du das SurrealDB Paket einbindest
  final String id;
  final String surrealql;
  final Map<String, dynamic>? params;

  // Mutable Properties, da sie sich via updateLocalState ändern
  String hash;
  dynamic tree; // Entspricht 'any' in TS, später 'IdTree'

  final QueryTimeToLive ttl;
  DateTime lastActiveAt;

  final IncantationMeta meta;

  // Private Felder für internen State
  Timer? _ttlTimer;
  late final int _ttlDurationMs;
  List<T>? _results;

  // Getters
  List<T>? get records => _results;
  String get tableName => meta.tableName;

  Incantation({
    required this.id,
    required this.surrealql,
    required this.hash,
    required this.ttl,
    required this.meta,
    this.params,
    this.tree,
    DateTime? lastActiveAt,
  }) : lastActiveAt = lastActiveAt ?? DateTime.now() {
    // Initialisiere die Dauer direkt im Konstruktor
    _ttlDurationMs = _parseDuration(ttl);
  }

  /// Aktualisiert den lokalen State mit neuen Daten vom Server oder Cache
  void updateLocalState(List<T> records, String hash, dynamic tree) {
    _results = records;
    this.hash = hash;
    this.tree = tree;
  }

  /// Stoppt den Heartbeat und räumt auf
  void destroy() {
    _stopTTLHeartbeat();
  }

  /// Startet den rekursiven Heartbeat Timer
  void startTTLHeartbeat(void Function() onHeartbeat) {
    if (_ttlTimer != null && _ttlTimer!.isActive) return;

    // Requirement: "call 10% before the TTL exceeds" => 90% of TTL.
    // .floor() rundet ab, um eine saubere Integer-Millisekunden-Zahl zu erhalten.
    final heartbeatTimeMs = (_ttlDurationMs * 0.9).floor();

    _ttlTimer = Timer(Duration(milliseconds: heartbeatTimeMs), () {
      onHeartbeat();
      // Rekursiver Aufruf, wie im TS Code
      // Wichtig: Timer ist one-shot, daher müssen wir ihn neu starten
      _stopTTLHeartbeat();
      startTTLHeartbeat(onHeartbeat);
    });
  }

  void _stopTTLHeartbeat() {
    _ttlTimer?.cancel();
    _ttlTimer = null;
  }

  /// Hilfsfunktion: Konvertiert TTL Enum in Millisekunden
  /// Portiert die Regex-Logik von `parseDuration` aus der TS-Datei.
  int _parseDuration(QueryTimeToLive ttl) {
    // Wir holen den String-Wert aus deiner Extension (z.B. "10m")
    final durationString = ttl.value;

    final regex = RegExp(r'^(\d+)([smh])$');
    final match = regex.firstMatch(durationString);

    if (match == null) {
      return 600000; // default 10m
    }

    final val = int.parse(match.group(1)!);
    final unit = match.group(2);

    switch (unit) {
      case 's':
        return val * 1000;
      case 'h':
        return val * 3600000;
      case 'm':
      default:
        return val * 60000;
    }
  }
}
