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
