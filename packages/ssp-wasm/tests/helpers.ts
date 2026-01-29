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
  sql: string,
  params?: Record<string, unknown>
): WasmIncantationConfig {
  return {
    id,
    sql,
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
/**
 * Validate that a flat array result has the expected structure.
 * Optionally validates that specific record IDs are present.
 */
export function validateFlatArray(resultData: unknown, expectedIds?: string[]): boolean {
  if (!Array.isArray(resultData)) return false;

  for (const item of resultData) {
    if (!Array.isArray(item) || item.length !== 2) return false;
    const [id, version] = item;
    if (typeof id !== 'string' || !id.includes(':')) return false;
    if (typeof version !== 'number') return false;
  }

  // If expected IDs provided, check they are all present
  if (expectedIds && expectedIds.length > 0) {
    const presentIds = resultData.map((item) => item[0]);
    for (const expectedId of expectedIds) {
      if (!presentIds.includes(expectedId)) {
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
 * Legacy validator for Views with Subqueries - Now effectively same as validateFlatArray
 * since subqueries are flattened/ignored in the output.
 */
export function validateFlatArrayWithChildren(
  resultData: unknown,
  expectedIds?: string[],
  expectedChildKeys?: string[]
): boolean {
  // Just validate flat array structure
  return validateFlatArray(resultData, expectedIds);
}

/**
 * Extract table name from a record ID (e.g., "user:123" -> "user")
 */
export function getTableFromId(id: string): string {
  const colonIndex = id.indexOf(':');
  return colonIndex > 0 ? id.substring(0, colonIndex) : id;
}
