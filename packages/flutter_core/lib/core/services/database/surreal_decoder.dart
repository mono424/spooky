import 'dart:convert';

sealed class SurrealValue {
  const SurrealValue();
  dynamic toJson();
}

class SurrealStrand extends SurrealValue {
  final String value;
  SurrealStrand(this.value);
  @override
  dynamic toJson() => value;
  @override
  String toString() => value;
}

class SurrealNumber extends SurrealValue {
  final num value;
  SurrealNumber(this.value);
  @override
  dynamic toJson() => value;
  @override
  String toString() => value.toString();
}

class SurrealBool extends SurrealValue {
  final bool value;
  SurrealBool(this.value);
  @override
  dynamic toJson() => value;
  @override
  String toString() => value.toString();
}

class SurrealThing extends SurrealValue {
  final String value;
  SurrealThing(this.value);
  @override
  dynamic toJson() => value;
  @override
  String toString() => value;
}

class SurrealDatetime extends SurrealValue {
  final DateTime value;
  SurrealDatetime(this.value);

  @override
  dynamic toJson() => value.toIso8601String();
  @override
  String toString() => value.toIso8601String();
}

class SurrealObject extends SurrealValue {
  final Map<String, SurrealValue> fields;
  SurrealObject(this.fields);

  @override
  dynamic toJson() => fields.map((k, v) => MapEntry(k, v.toJson()));
  @override
  String toString() => jsonEncode(toJson());
}

class SurrealArray extends SurrealValue {
  final List<SurrealValue> items;
  SurrealArray(this.items);

  @override
  dynamic toJson() => items.map((e) => e.toJson()).toList();
  @override
  String toString() => jsonEncode(toJson());
}

class SurrealNone extends SurrealValue {
  @override
  dynamic toJson() => null;
  @override
  String toString() => "null";
}

class SurrealDecoder {
  /// Die Hauptfunktion: Nimmt einen String und gibt ein SurrealValue zurück
  static SurrealValue decode(String input) {
    final dynamic jsonResponse = jsonDecode(input);
    return unwrap(jsonResponse);
  }

  static SurrealValue unwrap(dynamic data) {
    if (data is List) {
      return SurrealArray(data.map((item) => unwrap(item)).toList());
    }

    if (data is Map<String, dynamic>) {
      // Wenn die Map genau einen Key hat, der ein Surreal-Wrapper ist
      if (data.length == 1) {
        final key = data.keys.first;
        final value = data[key];

        switch (key) {
          case 'Strand':
            return SurrealStrand(value.toString());
          case 'Datetime':
            return SurrealDatetime(DateTime.parse(value.toString()));
          case 'Object':
            return unwrap(value);
          case 'Array':
            return unwrap(value);
          case 'Number':
            return SurrealNumber(num.parse(value.toString()));
          case 'Bool':
            return SurrealBool(value.toString().toLowerCase() == 'true');
          case 'Thing':
          case 'Id':
             // Handle Thing/Id which might be just a string
            return SurrealThing(value.toString());
          case 'None':
          case 'Null':
            return SurrealNone();
        }
      }

      // Falls es kein Wrapper ist, verarbeiten wir es als normales Objekt
      final Map<String, SurrealValue> fields = {};
      data.forEach((k, v) {
        fields[k] = unwrap(v);
      });
      return SurrealObject(fields);
    }

    // Fallback für primitive Werte, die nicht gewrappt sind
    return SurrealStrand(data.toString());
  }
}
