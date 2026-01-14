import { IdTree, IdTreeDiff } from '../../types.js';
import { parseRecordIdString } from '../utils/index.js';

export class TreeSyncer {
  private localTree: IdTree;
  private remoteTree: IdTree;

  constructor(localTree: IdTree, remoteTree: IdTree) {
    this.remoteTree = remoteTree;
    this.localTree = localTree;
  }

  /**
   * Updates the current local IdTree state.
   */
  update(local: IdTree) {
    this.localTree = local;
  }

  /**
   * Returns the difference between the new local IdTree and the remote IdTree.
   * Includes sets of added and removed nodes and the lowest level of updated nodes.
   */
  nextSet(): IdTreeDiff {
    return diffIdTree(this.localTree, this.remoteTree);
  }
}

export function diffIdTree(local: IdTree | null, remote: IdTree | null): IdTreeDiff {
  const { added, removed, updated } = diffIdTreeInt(local, remote);
  return {
    added: added.map(parseRecordIdString),
    removed: removed.map(parseRecordIdString),
    updated: updated.map(parseRecordIdString),
  };
}
/**
 * Recursive Internal Diff
 * Returns only the lowest level of difference.
 */
function diffIdTreeInt(
  local: IdTree | null,
  remote: IdTree | null
): { added: string[]; removed: string[]; updated: string[] } {
  const diff = { added: [] as string[], removed: [] as string[], updated: [] as string[] };

  // 1. Identical Hash (Fast Pass)
  if (local?.hash === remote?.hash) {
    return diff;
  }

  // 2. Compare Structural Children (Internal nodes without IDs, e.g. "comments" container)
  const localChildren = local?.children || {};
  const remoteChildren = remote?.children || {};
  const allChildKeys = new Set([...Object.keys(localChildren), ...Object.keys(remoteChildren)]);

  for (const key of allChildKeys) {
    const childDiff = diffIdTreeInt(localChildren[key] || null, remoteChildren[key] || null);
    mergeDiffs(diff, childDiff);
  }

  // 3. Compare Leaves (Nodes with IDs, e.g. Threads, Comments)
  // Note: Leaves here can be complex objects (trees themselves) per your example structure
  const localLeaves = mapLeavesById(local?.leaves);
  const remoteLeaves = mapLeavesById(remote?.leaves);
  const allLeafIds = new Set([...localLeaves.keys(), ...remoteLeaves.keys()]);

  for (const id of allLeafIds) {
    const lNode = localLeaves.get(id);
    const rNode = remoteLeaves.get(id);

    if (!lNode && rNode) {
      // Exists in Remote, missing in Local -> Added
      // Recursively collect all IDs in the new subtree to ensure nested records (authors, comments) are fetched
      diff.added.push(...collectAllIds(rNode));
    } else if (lNode && !rNode) {
      // Exists in Local, missing in Remote -> Removed
      diff.removed.push(id);
    } else if (lNode && rNode && lNode.hash !== rNode.hash) {
      // Both exist, Hash Mismatch -> DRILL DOWN
      // We must check if the change is "inside" this node (in its children/leaves)
      // or if the change is "intrinsic" to this node (e.g. title text changed).

      const deepDiff = diffIdTreeInt(lNode, rNode);
      const hasDeepChanges =
        deepDiff.added.length > 0 || deepDiff.removed.length > 0 || deepDiff.updated.length > 0;

      if (hasDeepChanges) {
        // The change was found deeper in the tree.
        // Return the deep changes and IGNORE the current parent ID.
        mergeDiffs(diff, deepDiff);
      } else {
        // No children explained the hash change.
        // The change is intrinsic to this node.
        diff.updated.push(id);
      }
    }
  }

  return diff;
}

// --- Helpers ---

function mergeDiffs(
  target: { added: string[]; removed: string[]; updated: string[] },
  source: { added: string[]; removed: string[]; updated: string[] }
) {
  target.added.push(...source.added);
  target.removed.push(...source.removed);
  target.updated.push(...source.updated);
}

// Helper to map an array of leaf nodes by their ID for easy lookup
// We treat "leaves" in the JSON as full IdTrees since they have children/hash/id
function mapLeavesById(
  leaves?: { id: string; hash: string; children?: any; leaves?: any }[]
): Map<string, any> {
  const map = new Map<string, any>();
  if (!leaves) return map;
  for (const leaf of leaves) {
    if (leaf.id) {
      map.set(leaf.id, leaf);
    }
  }
  return map;
}

/**
 * Recursively collect all IDs from a node and its children/leaves.
 * Used when a parent node is added, ensuring we fetch all its descendants.
 */
function collectAllIds(node: any): string[] {
  const ids: string[] = [];
  if (node.id) ids.push(node.id);

  if (node.children) {
    // children is an Object (or Map-like)
    Object.values(node.children).forEach((child: any) => {
      ids.push(...collectAllIds(child));
    });
  }

  if (node.leaves) {
    // leaves is an Array of LeafItems
    node.leaves.forEach((leaf: any) => {
      ids.push(...collectAllIds(leaf));
    });
  }
  return ids;
}
