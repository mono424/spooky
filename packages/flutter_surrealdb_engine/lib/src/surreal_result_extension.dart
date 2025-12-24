import 'package:flutter_surrealdb_engine/src/rust/lib.dart';

extension SurrealResultToJson on SurrealResult {
  Map<String, dynamic> toJson() => {
    'result': result,
    'status': status,
    'time': time,
  };
}

extension SurrealResultListToJson on List<SurrealResult> {
  List<Map<String, dynamic>> toJson() => map((e) => e.toJson()).toList();
}
