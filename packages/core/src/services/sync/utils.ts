import { IdTree, IdTreeDiff } from '../../types.js';
import { parseRecordIdString } from '../utils.js';

export function diffIdTree(local: IdTree | null, remote: IdTree | null): IdTreeDiff {
  const { added, removed, updated } = diffIdTreeInt(local, remote);
  return {
    added: added.map(parseRecordIdString),
    removed: removed.map(parseRecordIdString),
    updated: updated.map(parseRecordIdString),
  };
}

function diffIdTreeInt(
  local: IdTree | null,
  remote: IdTree | null
): { added: string[]; removed: string[]; updated: string[] } {
  // 1. Identical Hash (Fast Pass)
  if (local?.hash === remote?.hash) {
    return { added: [], removed: [], updated: [] };
  }

  // 2. Both are Internal Nodes (Structure Match) -> Recurse
  if (local?.children && remote?.children) {
    return diffInternalNodes(local.children, remote.children);
  }

  // 3. Structure Mismatch (Leaf, Null, or Mixed) -> Flatten & Diff
  return diffLists(flattenIdTree(local), flattenIdTree(remote));
}

function diffInternalNodes(
  localChildren: Record<string, IdTree>,
  remoteChildren: Record<string, IdTree>
) {
  const added: string[] = [];
  const removed: string[] = [];
  const updated: string[] = [];
  const allKeys = new Set([...Object.keys(localChildren), ...Object.keys(remoteChildren)]);

  for (const key of allKeys) {
    const {
      added: a,
      removed: r,
      updated: u,
    } = diffIdTreeInt(localChildren[key] || null, remoteChildren[key] || null);
    added.push(...a);
    removed.push(...r);
    updated.push(...u);
  }
  return { added, removed, updated };
}

export function flattenIdTree(node: IdTree | null): { id: string; hash: string }[] {
  if (!node) return [];
  const leaves: { id: string; hash: string }[] = [];
  collectLeaves(node, leaves);
  return leaves;
}

function collectLeaves(node: IdTree, list: { id: string; hash: string }[]) {
  if (node.leaves) {
    list.push(...node.leaves);
  }
  if (node.children) {
    for (const child of Object.values(node.children)) {
      collectLeaves(child, list);
    }
  }
}

function diffLists(
  localLeaves: { id: string; hash: string }[],
  remoteLeaves: { id: string; hash: string }[]
): { added: string[]; removed: string[]; updated: string[] } {
  const localMap = new Map(localLeaves.map((l) => [l.id, l.hash]));
  const remoteMap = new Map(remoteLeaves.map((l) => [l.id, l.hash]));

  const added: string[] = [];
  const removed: string[] = [];
  const updated: string[] = [];

  for (const [id, hash] of remoteMap) {
    if (!localMap.has(id)) {
      added.push(id);
    } else if (localMap.get(id) !== hash) {
      updated.push(id);
    }
  }

  for (const id of localMap.keys()) {
    if (!remoteMap.has(id)) {
      removed.push(id);
    }
  }

  return { added, removed, updated };
}
