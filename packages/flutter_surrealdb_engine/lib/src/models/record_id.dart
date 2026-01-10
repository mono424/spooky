/// A SurrealDB Record ID consisting of a Table and a Key.
///
/// Can be serialized to JSON as `{"table": "...", "key": "..."}`
/// which is compatible with the `queryTyped` middleware of the engine.
class RecordId {
  final String table;
  final dynamic key;

  const RecordId({required this.table, required this.key});

  /// standard `user:123` string format
  @override
  String toString() => '$table:$key';

  /// Parses a "table:key" string into a RecordId
  factory RecordId.fromString(String id) {
    final parts = id.split(':');
    if (parts.length < 2) {
      throw FormatException("Invalid RecordId string: $id");
    }
    final table = parts[0];
    // Re-join the rest in case key contains ':'
    final key = parts.sublist(1).join(':');
    return RecordId(table: table, key: key);
  }

  /// Converts to the specific JSON structure expected by the Rust middleware
  Map<String, dynamic> toJson() => {'table': table, 'key': key};

  factory RecordId.fromJson(Map<String, dynamic> json) {
    return RecordId(table: json['table'] as String, key: json['key']);
  }

  @override
  bool operator ==(Object other) {
    if (identical(this, other)) return true;
    return other is RecordId && other.table == table && other.key == key;
  }

  @override
  int get hashCode => table.hashCode ^ key.hashCode;
}
