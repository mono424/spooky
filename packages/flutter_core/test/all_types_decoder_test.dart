import 'package:flutter_core/core/services/database/surreal_decoder.dart';
import 'package:test/test.dart';

void main() {
  group('SurrealDecoder Comprehensive Tests', () {
    // --- Primitives ---

    test('Null / None', () {
      expect(SurrealDecoder.decodeNative('"None"'), null);
      expect(SurrealDecoder.decodeNative('{"None": null}'), null);
      expect(SurrealDecoder.decodeNative('{"Null": null}'), null);
      // Should handle raw null if it ever happens
      expect(SurrealDecoder.decodeNative('null'), null);
    });

    test('Bool', () {
      expect(SurrealDecoder.decodeNative('{"Bool": true}'), true);
      expect(SurrealDecoder.decodeNative('{"Bool": false}'), false);
    });

    test('Number', () {
      expect(SurrealDecoder.decodeNative('{"Number": 100}'), 100);
      expect(SurrealDecoder.decodeNative('{"Number": 100.5}'), 100.5);
      expect(SurrealDecoder.decodeNative('{"Number": -50}'), -50);
      // Scientific notation might come as number or string depending on JSON parser, but usually Number
    });

    test('String / Strand', () {
      expect(
        SurrealDecoder.decodeNative('{"String": "hello world"}'),
        "hello world",
      );
      expect(SurrealDecoder.decodeNative('{"Strand": "foo bar"}'), "foo bar");
    });

    // --- Complex Types ---

    test('Array', () {
      final input = '{"Array": [{"String": "one"}, {"Number": 2}, "None"]}';
      final result = SurrealDecoder.decodeNative(input);
      expect(result, ["one", 2, null]);
    });

    test('Object', () {
      final input =
          '{"Object": {"key1": {"String": "val1"}, "key2": {"Number": 2}}}';
      final result = SurrealDecoder.decodeNative(input);
      expect(result, {"key1": "val1", "key2": 2});
    });

    // --- SurrealDB Specific Types ---

    test('Datetime', () {
      // ISO 8601 String
      const dateStr = "2023-10-27T10:00:00.000Z";
      final input = '{"Datetime": "$dateStr"}';
      final result = SurrealDecoder.decodeNative(input);
      expect(result, isA<DateTime>());
      expect((result as DateTime).toUtc().toIso8601String(), dateStr);
    });

    test('RecordId / Thing', () {
      // Standard string ID
      expect(
        SurrealDecoder.decodeNative(
          '{"RecordId": {"table": "user", "key": {"String": "123"}}}',
        ),
        "user:123",
      );
      // numeric ID
      expect(
        SurrealDecoder.decodeNative(
          '{"RecordId": {"table": "post", "key": {"Number": 999}}}',
        ),
        "post:999",
      );
      // simple Thing wrapper
      expect(SurrealDecoder.decodeNative('{"Thing": "user:456"}'), "user:456");
      expect(SurrealDecoder.decodeNative('{"Id": "user:789"}'), "user:789");
    });

    // --- Extended Types (Attempted) ---
    // Note: If these aren't explicitly handled in the switch, they might fall through
    // to the "length=1 Map" logic or be treated as Objects.
    // We want to see how the current decoder handles them.

    test('Uuid', () {
      const uuid = "550e8400-e29b-41d4-a716-446655440000";
      // Assuming Rust outputs {"Uuid": "..."}
      final input = '{"Uuid": "$uuid"}';
      final result = SurrealDecoder.decodeNative(input);
      // If unhandled, it comes back as SurrealStrand via fallback? Or SurrealObject?
      // With current logic: length=1 map, key='Uuid' -> switch default -> falls out -> treated as Object {"Uuid": "value"}
      // Ideally we might want it to be just the string.
      print("Uuid result: $result");
      // Expectation depends on desired behavior. For now, let's see what happens.
    });

    test('Duration', () {
      const duration = "1h30m";
      // Assuming Rust outputs {"Duration": "..."}
      final input = '{"Duration": "$duration"}';
      final result = SurrealDecoder.decodeNative(input);
      print("Duration result: $result");
    });

    test('Geometry (Point)', () {
      // {"Geometry": {"Point": [10.0, 20.0]}}
      final input =
          '{"Geometry": {"Point": [{"Number": 10.0}, {"Number": 20.0}]}}';
      final result = SurrealDecoder.decodeNative(input);
      print("Geometry result: $result");
    });

    test('Table', () {
      // {"Table": "user"}
      final input = '{"Table": "user"}';
      final result = SurrealDecoder.decodeNative(input);
      expect(result, anything); // Just checking it doesn't crash
    });

    test('Bytes', () {
      // {"Bytes": "..."} usually base64?
      final input = '{"Bytes": "SGVsbG8="}';
      final result = SurrealDecoder.decodeNative(input);
      expect(result, anything);
    });
  });
}
