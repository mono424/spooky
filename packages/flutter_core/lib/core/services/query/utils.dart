import 'dart:convert';
import '../database/surreal_decoder.dart';

/// Hilft beim Extrahieren des tatsächlichen Ergebnisses aus dem Engine-JSON-String.
/// Die Engine gibt meistens so etwas zurück: `[{"status": "OK", "time": "...", "result": ...}]`
dynamic extractResult(String jsonString) {
  final List<dynamic> rawList = jsonDecode(jsonString);
  if (rawList.isEmpty) return null;

  // Wir nehmen das erste Ergebnis (da wir meist nur 1 Query senden)
  final firstQuery = rawList[0];

  // Check if standard response object (Map) or raw result
  if (firstQuery is Map<String, dynamic>) {
    if (firstQuery['status'] != 'OK') {
      throw Exception(
        'SurrealDB Query Error: ${firstQuery['detail'] ?? jsonEncode(firstQuery)}',
      );
    }
    final rawResult = firstQuery['result'];
    if (rawResult == null) return null;
    final wrapped = SurrealDecoder.unwrap(rawResult);
    return wrapped.v;
  } else {
    // Treat as direct result (e.g. RETURN "foo" -> ["foo"])
    return firstQuery;
  }
}
