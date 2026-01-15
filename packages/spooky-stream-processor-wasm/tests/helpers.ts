/**
 * Test helpers for spooky-stream-processor-wasm E2E tests.
 * Mirrors the patterns from packages/spooky-stream-processor/tests/common/mod.rs
 */

import type { WasmIncantationConfig } from '../pkg/spooky_stream_processor_wasm';

let counter = 0;

/**
 * Generate a unique ID (simple incrementing counter for deterministic tests)
 */
export function generateId(): string {
  return `${Date.now()}_${++counter}`;
}

/**
 * Create an author record
 */
export function makeAuthorRecord(name: string): { id: string; record: Record<string, unknown> } {
  const idRaw = generateId();
  const id = `author:${idRaw}`;
  const record = {
    id,
    name,
    type: 'author',
  };
  return { id, record };
}

/**
 * Create a thread record with reference to author
 */
export function makeThreadRecord(
  title: string,
  authorId: string
): { id: string; record: Record<string, unknown> } {
  const idRaw = generateId();
  const id = `thread:${idRaw}`;
  const record = {
    id,
    title,
    author: authorId,
    type: 'thread',
  };
  return { id, record };
}

/**
 * Create a comment record with references to thread and author
 */
export function makeCommentRecord(
  text: string,
  threadId: string,
  authorId: string
): { id: string; record: Record<string, unknown> } {
  const idRaw = generateId();
  const id = `comment:${idRaw}`;
  const record = {
    id,
    text,
    thread: threadId,
    author: authorId,
    type: 'comment',
  };
  return { id, record };
}

/**
 * Create a user record (for simple view tests)
 */
export function makeUserRecord(
  username: string,
  email: string
): { id: string; record: Record<string, unknown> } {
  const idRaw = generateId();
  const id = `user:${idRaw}`;
  const record = {
    id,
    username,
    email,
    type: 'user',
  };
  return { id, record };
}

/**
 * Create a view config for registering with the processor
 */
export function createViewConfig(
  id: string,
  surrealQL: string,
  params?: Record<string, unknown>
): WasmIncantationConfig {
  return {
    id,
    surrealQL,
    params,
    clientId: 'test-client',
    ttl: '3600s',
    lastActiveAt: new Date().toISOString(),
  };
}

/**
 * Validate that a hash tree has the expected structure.
 * Optionally validates that specific record IDs are present.
 */
export function validateHashTree(tree: unknown, expectedIds?: string[]): boolean {
  if (tree === null || tree === undefined) return false;
  if (typeof tree !== 'object') return false;

  const t = tree as { hash?: unknown; leaves?: unknown[] };

  // Check root hash exists and is valid hex format (64 chars for blake3)
  if (typeof t.hash !== 'string' || !isValidHashFormat(t.hash)) {
    return false;
  }

  // Check leaves array exists
  if (!Array.isArray(t.leaves)) {
    return false;
  }

  // Validate each leaf
  for (const leaf of t.leaves) {
    if (!isValidLeaf(leaf)) {
      return false;
    }
  }

  // If expected IDs provided, check they are all present
  if (expectedIds && expectedIds.length > 0) {
    const leafIds = t.leaves.map((leaf: any) => leaf.id);
    for (const expectedId of expectedIds) {
      if (!leafIds.includes(expectedId)) {
        return false;
      }
    }
  }

  return true;
}

/**
 * Check if a string is a valid hash format (64-char lowercase hex for blake3)
 */
function isValidHashFormat(hash: string): boolean {
  return /^[a-f0-9]{64}$/.test(hash);
}

/**
 * Validate a leaf node in the hash tree
 */
function isValidLeaf(leaf: unknown, requireChildren = false): boolean {
  if (typeof leaf !== 'object' || leaf === null) return false;

  const l = leaf as { id?: unknown; hash?: unknown; children?: unknown };

  // Must have valid id (format: table:id)
  if (typeof l.id !== 'string' || !l.id.includes(':')) {
    return false;
  }

  // Must have valid hash
  if (typeof l.hash !== 'string' || !isValidHashFormat(l.hash)) {
    return false;
  }

  // children is optional, but if present must be an object
  if (l.children !== undefined && typeof l.children !== 'object') {
    return false;
  }

  // If children are required (for joined views), check they exist and are non-empty
  if (requireChildren) {
    if (!l.children || Object.keys(l.children as object).length === 0) {
      console.warn(`[validateHashTree] Leaf ${l.id} has empty children, expected joined data`);
      return false;
    }
  }

  return true;
}

/**
 * Validate hash tree for views WITH subqueries/joins.
 * This checks that leaves have populated children (the joined data).
 */
export function validateHashTreeWithChildren(
  tree: unknown,
  expectedIds?: string[],
  expectedChildKeys?: string[]
): boolean {
  if (tree === null || tree === undefined) return false;
  if (typeof tree !== 'object') return false;

  const t = tree as { hash?: unknown; leaves?: unknown[] };

  // Check root hash exists and is valid hex format
  if (typeof t.hash !== 'string' || !isValidHashFormat(t.hash)) {
    return false;
  }

  // Check leaves array exists
  if (!Array.isArray(t.leaves)) {
    return false;
  }

  // Validate each leaf (requiring non-empty children for joined views)
  for (const leaf of t.leaves) {
    if (!isValidLeaf(leaf, true)) {
      return false;
    }

    // If expected child keys provided, check they exist
    if (expectedChildKeys && expectedChildKeys.length > 0) {
      const children = (leaf as any).children as Record<string, unknown>;
      for (const key of expectedChildKeys) {
        if (!(key in children)) {
          console.warn(`[validateHashTree] Leaf missing expected child key: ${key}`);
          return false;
        }
      }
    }
  }

  // If expected IDs provided, check they are all present
  if (expectedIds && expectedIds.length > 0) {
    const leafIds = t.leaves.map((leaf: any) => leaf.id);
    for (const expectedId of expectedIds) {
      if (!leafIds.includes(expectedId)) {
        return false;
      }
    }
  }

  return true;
}

/**
 * Extract table name from a record ID (e.g., "user:123" -> "user")
 */
export function getTableFromId(id: string): string {
  const colonIndex = id.indexOf(':');
  return colonIndex > 0 ? id.substring(0, colonIndex) : id;
}
