import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import '../../types.dart';

// --- Public API ---

IdTreeDiff diffIdTree(IdTree? local, IdTree? remote) {
  final result = diffIdTreeInt(local, remote);
  return IdTreeDiff(
    added: result.added,
    removed: result.removed,
    updated: result.updated,
  );
}

List<LeafNode> flattenIdTree(IdTree? node) => _flatten(node);

// --- Internal Implementation ---

class _Tags {
  final List<String> added;
  final List<String> removed;
  final List<String> updated;

  _Tags({required this.added, required this.removed, required this.updated});

  factory _Tags.empty() => _Tags(added: [], removed: [], updated: []);
}

_Tags diffIdTreeInt(IdTree? local, IdTree? remote) {
  // 1. Identical Hash (Fast Pass)
  if (local?.hash == remote?.hash) {
    return _Tags.empty();
  }

  // 2. Both are Internal Nodes (Structure Match) -> Recurse
  if (local != null &&
      local.children != null &&
      remote != null &&
      remote.children != null) {
    return _diffInternalNodes(local.children!, remote.children!);
  }

  // 3. Structure Mismatch (Leaf, Null, or Mixed) -> Flatten & Diff
  return _diffLists(_flatten(local), _flatten(remote));
}

_Tags _diffInternalNodes(
  Map<String, IdTree> localChildren,
  Map<String, IdTree> remoteChildren,
) {
  final added = <String>[];
  final removed = <String>[];
  final updated = <String>[];

  final allKeys = {...localChildren.keys, ...remoteChildren.keys};

  for (final key in allKeys) {
    final result = diffIdTreeInt(localChildren[key], remoteChildren[key]);
    added.addAll(result.added);
    removed.addAll(result.removed);
    updated.addAll(result.updated);
  }
  return _Tags(added: added, removed: removed, updated: updated);
}

List<LeafNode> _flatten(IdTree? node) {
  if (node == null) return [];
  final leaves = <LeafNode>[];
  _collectLeaves(node, leaves);
  return leaves;
}

void _collectLeaves(IdTree node, List<LeafNode> list) {
  if (node.leaves != null) {
    list.addAll(node.leaves!);
  }
  if (node.children != null) {
    for (final child in node.children!.values) {
      _collectLeaves(child, list);
    }
  }
}

_Tags _diffLists(List<LeafNode> localLeaves, List<LeafNode> remoteLeaves) {
  final localMap = {for (var l in localLeaves) l.id: l.hash};
  final remoteMap = {for (var l in remoteLeaves) l.id: l.hash};

  final added = <String>[];
  final removed = <String>[];
  final updated = <String>[];

  // Identifiziere Added und Updated
  for (final entry in remoteMap.entries) {
    final id = entry.key;
    final hash = entry.value;

    if (!localMap.containsKey(id)) {
      added.add(id);
    } else if (localMap[id] != hash) {
      updated.add(id);
    }
  }

  // Identifiziere Removed
  for (final id in localMap.keys) {
    if (!remoteMap.containsKey(id)) {
      removed.add(id);
    }
  }

  return _Tags(added: added, removed: removed, updated: updated);
}

/// Recursively flattens nested records.
/// [results]: List of records to flatten.
/// [relationshipMap]: Mapping of table -> fields to traverse.
List<Map<String, dynamic>> flattenRecords(
  List<Map<String, dynamic>> results,
  Map<String, Set<String>> relationshipMap, [
  Set<String>? visited,
  List<Map<String, dynamic>>? flattened,
]) {
  visited ??= {};
  flattened ??= [];

  for (final record in results) {
    // 1. Identify the Record by ID
    String? recordIdStr;
    String? tableName;

    dynamic idField = record['id'];
    if (idField is String) {
      recordIdStr = idField;
      final parts = idField.split(':');
      if (parts.length > 1) tableName = parts[0];
    } else if (idField is RecordId) {
      recordIdStr = idField.toString();
      tableName = idField.table;
    }

    if (recordIdStr != null) {
      if (visited.contains(recordIdStr)) continue;
      visited.add(recordIdStr);
    }

    // 2. Clone to avoid mutation
    final processedRecord = Map<String, dynamic>.from(record);

    // 3. Handle Relationships
    if (tableName != null && relationshipMap.containsKey(tableName)) {
      final validFields = relationshipMap[tableName]!;

      for (final key in validFields) {
        if (!processedRecord.containsKey(key)) continue;

        final value = processedRecord[key];

        if (value is List) {
          // List of records?
          final nestedList = value.whereType<Map<String, dynamic>>().toList();
          if (nestedList.isNotEmpty) {
            flattenRecords(nestedList, relationshipMap, visited, flattened);
            // Replace with IDs
            processedRecord[key] = nestedList.map((r) => r['id']).toList();
          }
        } else if (value is Map<String, dynamic>) {
          // Single nested record
          final nestedId = value['id'];
          if (nestedId != null) {
            flattenRecords([value], relationshipMap, visited, flattened);
            processedRecord[key] = nestedId;
          }
        }
      }
    }

    flattened.add(processedRecord);
  }

  return flattened;
}
