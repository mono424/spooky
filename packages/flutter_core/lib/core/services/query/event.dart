import '../events/main.dart';
import '../../types.dart';

// Falls du eine spezielle RecordId Klasse hast, importiere sie.
// Hier nutze ich String als Platzhalter für RecordId<string>.

/// Basisklasse für alle Query Events.
abstract class QueryEvent extends BaseEvent<dynamic> {
  QueryEvent(String type, dynamic payload) : super(type, payload);
}

// --- 1. IncantationInitialized ---

class IncantationInitializedPayload {
  final String incantationId; // entspricht RecordId<string>
  final String surrealql;
  final Map<String, dynamic>? params;
  final QueryTimeToLive ttl;

  IncantationInitializedPayload({
    required this.incantationId,
    required this.surrealql,
    required this.ttl,
    this.params,
  });
}

class IncantationInitialized extends QueryEvent {
  static const String typeName = 'QUERY_INCANTATION_INITIALIZED';

  // Override sorgt dafür, dass payload beim Zugriff den richtigen Typ hat
  @override
  final IncantationInitializedPayload payload;

  IncantationInitialized(this.payload) : super(typeName, payload);
}

// --- 2. IncantationRemoteHashUpdate ---

class IncantationRemoteHashUpdatePayload {
  final String incantationId;
  final String surrealql;
  final String localHash;
  final dynamic localTree; // 'any' in TS -> dynamic in Dart
  final String remoteHash;
  final dynamic remoteTree;

  IncantationRemoteHashUpdatePayload({
    required this.incantationId,
    required this.surrealql,
    required this.localHash,
    required this.localTree,
    required this.remoteHash,
    required this.remoteTree,
  });
}

class IncantationRemoteHashUpdate extends QueryEvent {
  static const String typeName = 'QUERY_INCANTATION_REMOTE_HASH_UPDATE';

  @override
  final IncantationRemoteHashUpdatePayload payload;

  IncantationRemoteHashUpdate(this.payload) : super(typeName, payload);
}

// --- 3. IncantationTTLHeartbeat ---

class IncantationTTLHeartbeatPayload {
  final String incantationId;
  IncantationTTLHeartbeatPayload({required this.incantationId});
}

class IncantationTTLHeartbeat extends QueryEvent {
  static const String typeName = 'QUERY_INCANTATION_TTL_HEARTBEAT';

  @override
  final IncantationTTLHeartbeatPayload payload;

  IncantationTTLHeartbeat(this.payload) : super(typeName, payload);
}

// --- 4. IncantationCleanup ---

class IncantationCleanupPayload {
  final String incantationId;
  IncantationCleanupPayload({required this.incantationId});
}

class IncantationCleanup extends QueryEvent {
  static const String typeName = 'QUERY_INCANTATION_CLEANUP';

  @override
  final IncantationCleanupPayload payload;

  IncantationCleanup(this.payload) : super(typeName, payload);
}

// --- 5. IncantationIncomingRemoteUpdate ---

class IncantationIncomingRemoteUpdatePayload {
  final String incantationId;
  final String remoteHash;
  final dynamic remoteTree;
  final List<Map<String, dynamic>> records;

  IncantationIncomingRemoteUpdatePayload({
    required this.incantationId,
    required this.remoteHash,
    required this.remoteTree,
    required this.records,
  });
}

class IncantationIncomingRemoteUpdate extends QueryEvent {
  static const String typeName = 'QUERY_INCANTATION_INCOMING_REMOTE_UPDATE';

  @override
  final IncantationIncomingRemoteUpdatePayload payload;

  IncantationIncomingRemoteUpdate(this.payload) : super(typeName, payload);
}

// --- 6. IncantationUpdated ---

class IncantationUpdatedPayload {
  final String incantationId;
  final List<Map<String, dynamic>> records;

  IncantationUpdatedPayload({
    required this.incantationId,
    required this.records,
  });
}

class IncantationUpdated extends QueryEvent {
  static const String typeName = 'QUERY_INCANTATION_UPDATED';

  @override
  final IncantationUpdatedPayload payload;

  IncantationUpdated(this.payload) : super(typeName, payload);
}
