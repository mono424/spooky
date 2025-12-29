import 'dart:convert';
import '../database/surreal_decoder.dart';

/// Hilft beim Extrahieren des tatsächlichen Ergebnisses aus dem Engine-JSON-String.
/// Die Engine gibt meistens so etwas zurück: `[{"status": "OK", "time": "...", "result": ...}]`
dynamic extractResult(String jsonString) {
  final List<dynamic> rawList = jsonDecode(jsonString);
  if (rawList.isEmpty) return null;

  // Wir nehmen das erste Ergebnis (da wir meist nur 1 Query senden)
  final firstQuery = rawList[0];

  if (firstQuery['status'] != 'OK') {
    throw Exception(
      'SurrealDB Query Error: ${firstQuery['detail'] ?? "Unknown"}',
    );
  }

  // Das eigentliche 'result' Feld holen und via SurrealDecoder in Dart-Objekte wandeln
  final rawResult = firstQuery['result'];

  // Wenn result null ist, gib null zurück
  if (rawResult == null) return null;

  // SurrealDecoder nutzen, um Wrappings wie "Strand", "Number" zu entfernen
  final wrapped = SurrealDecoder.unwrap(rawResult);
  return wrapped.v;
}
