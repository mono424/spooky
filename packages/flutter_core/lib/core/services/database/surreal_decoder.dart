import 'dart:convert';
import 'dart:typed_data';

sealed class SurrealValue {
  const SurrealValue();

  dynamic toJson();
  dynamic get v;
}

class SurrealStrand extends SurrealValue {
  final String value;
  const SurrealStrand(this.value);

  @override
  dynamic toJson() => value;

  @override
  dynamic get v => value;

  @override
  String toString() => jsonEncode(value);
}

class SurrealNumber extends SurrealValue {
  final num value;
  const SurrealNumber(this.value);

  @override
  dynamic toJson() => value;
  @override
  dynamic get v => value;
  @override
  String toString() => value.toString();
}

class SurrealBool extends SurrealValue {
  final bool value;
  const SurrealBool(this.value);

  @override
  dynamic toJson() => value;
  @override
  dynamic get v => value;
  @override
  String toString() => value.toString();
}

class SurrealThing extends SurrealValue {
  final String value;
  const SurrealThing(this.value);

  @override
  dynamic toJson() => value;
  @override
  dynamic get v => value;
  @override
  String toString() => value.toString();
}

class SurrealDatetime extends SurrealValue {
  final DateTime value;
  const SurrealDatetime(this.value);

  @override
  dynamic toJson() => value.toIso8601String();
  @override
  dynamic get v => value;
  @override
  String toString() => jsonEncode(value.toIso8601String());
}

class SurrealObject extends SurrealValue {
  final Map<String, SurrealValue> fields;
  const SurrealObject(this.fields);

  @override
  dynamic toJson() => fields.map((k, v) => MapEntry(k, v.toJson()));
  @override
  dynamic get v => fields.map((k, val) => MapEntry(k, val.v));
  @override
  String toString() => jsonEncode(toJson());
}

class SurrealArray extends SurrealValue {
  final List<SurrealValue> items;
  const SurrealArray(this.items);

  @override
  dynamic toJson() => items.map((e) => e.toJson()).toList();
  @override
  dynamic get v => items.map((e) => e.v).toList();
  @override
  String toString() => jsonEncode(toJson());
}

class SurrealNone extends SurrealValue {
  const SurrealNone();
  @override
  dynamic toJson() => null;

  @override
  dynamic get v => null; // Gibt echtes null zurück
}

class SurrealBytes extends SurrealValue {
  final Uint8List value;
  const SurrealBytes(this.value);

  @override
  dynamic toJson() => base64Encode(value);
  @override
  dynamic get v => value;
  @override
  String toString() => jsonEncode(base64Encode(value));
}

class SurrealUuid extends SurrealValue {
  final String value;
  const SurrealUuid(this.value);

  @override
  dynamic toJson() => value;
  @override
  dynamic get v => value;
  @override
  String toString() => jsonEncode(value);
}

class SurrealDecoder {
  /// Gibt dir direkt "echte" Dart Objekte zurück (List, Map, String...)
  /// Du musst dich nicht mehr mit SurrealArray etc. herumschlagen.
  static dynamic decodeNative(String input, {bool removeNulls = false}) {
    final wrapper = decode(input, removeNulls: removeNulls);
    return wrapper.v;
  }

  static SurrealValue decode(String input, {bool removeNulls = false}) {
    final dynamic jsonResponse = jsonDecode(input);
    return unwrap(jsonResponse, removeNulls: removeNulls);
  }

  static SurrealValue unwrap(dynamic data, {bool removeNulls = false}) {
    if (data == "None") return const SurrealNone();

    if (data is List) {
      final List<SurrealValue> items = [];
      for (var item in data) {
        final val = unwrap(item, removeNulls: removeNulls);
        if (removeNulls && val is SurrealNone) continue;
        items.add(val);
      }
      return SurrealArray(items);
    }

    if (data is Map<String, dynamic>) {
      if (data.length == 1) {
        final key = data.keys.first;
        final value = data[key];
        switch (key) {
          case 'Strand':
          case 'String':
            return SurrealStrand(value.toString());
          case 'Datetime':
            return SurrealDatetime(DateTime.parse(value.toString()));
          case 'Object':
            return unwrap(value, removeNulls: removeNulls);
          case 'Array':
            return unwrap(value, removeNulls: removeNulls);
          case 'Number':
            return SurrealNumber(num.parse(value.toString()));
          case 'Bool':
            return SurrealBool(value.toString().toLowerCase() == 'true');
          case 'Thing':
          case 'Id':
            return SurrealUuid(value.toString());
          case 'RecordId':
            final keyPart = unwrap(value['key'], removeNulls: removeNulls);
            final tablePart = value['table'];
            return SurrealThing("$tablePart:${keyPart.v}");
          case 'None':
          case 'Null':
            return const SurrealNone();
          case 'Uuid':
            return SurrealThing(value.toString()); // Treat UUIDs as strings/IDs
          case 'Duration':
            return SurrealStrand(value.toString());
          case 'Table':
            return SurrealStrand(value.toString());
          case 'Bytes':
            return SurrealBytes(base64Decode(value.toString()));
          case 'Geometry':
            // Recursively unwrap geometry content (e.g. Point, LineString)
            return unwrap(value, removeNulls: removeNulls);
          case 'Point':
          case 'LineString':
          case 'Polygon':
          case 'MultiPoint':
          case 'MultiLineString':
          case 'MultiPolygon':
          case 'GeometryCollection':
            // These are GeoJSON-like structures inside Geometry
            // We unwrap them. If it's a list (coordinates), recursively unwrap.
            return unwrap(value, removeNulls: removeNulls);
        }
      }
      final Map<String, SurrealValue> fields = {};
      data.forEach((k, v) {
        final unwrapped = unwrap(v, removeNulls: removeNulls);
        if (removeNulls && unwrapped is SurrealNone) return;
        fields[k] = unwrapped;
      });
      return SurrealObject(fields);
    }
    if (data == null) return const SurrealNone();
    return SurrealStrand(data.toString());
  }
}
