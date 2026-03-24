/**
 * Test helpers for sp00ky-stream-processor-wasm E2E tests.
 * Mirrors the patterns from packages/sp00ky-stream-processor/tests/common/mod.rs
 */

import type { WasmViewConfig } from '../pkg/ssp_wasm';

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
): WasmViewConfig {
  return {
    id,
    surql: sql,
    params,
    clientId: 'test-client',
    ttl: '3600s',
    lastActiveAt: new Date().toISOString(),
  };
}

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
 * Create a product record (for WHERE clause and ORDER BY tests)
 */
export function makeProductRecord(
  name: string,
  price: number,
  category: string
): { id: string; record: Record<string, unknown> } {
  const idRaw = generateId();
  const id = `product:${idRaw}`;
  const record = {
    id,
    name,
    price,
    category,
    type: 'product',
  };
  return { id, record };
}

/**
 * Create a user record with numeric fields (for filtering/ordering tests)
 */
export function makeUserRecordExtended(
  username: string,
  email: string,
  age: number,
  level: number
): { id: string; record: Record<string, unknown> } {
  const idRaw = generateId();
  const id = `user:${idRaw}`;
  const record = {
    id,
    username,
    email,
    age,
    level,
    type: 'user',
  };
  return { id, record };
}

