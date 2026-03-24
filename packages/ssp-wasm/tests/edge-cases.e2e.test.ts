/**
 * Edge Case E2E Test Suite for ssp-wasm
 *
 * Tests boundary conditions, error handling, and untested API methods:
 * - save_state / load_state
 * - unregister_view
 * - UPDATE operation
 * - WHERE clause filtering
 * - ORDER BY / LIMIT
 * - Multiple concurrent views
 * - Parameterized queries
 * - Empty/invalid inputs
 * - Duplicate/non-existent operations
 * - Version tracking and delta structure
 */

import { describe, it, expect, beforeAll } from 'vitest';
import { readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { initSync, Sp00kyProcessor } from '../pkg/ssp_wasm.js';
import type { WasmViewUpdate } from '../pkg/ssp_wasm';
import {
  makeUserRecord,
  makeUserRecordExtended,
  makeProductRecord,
  makeAuthorRecord,
  makeThreadRecord,
  makeCommentRecord,
  createViewConfig,
  validateFlatArray,
  generateId,
} from './helpers';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

beforeAll(() => {
  const wasmPath = join(__dirname, '../pkg/ssp_wasm_bg.wasm');
  const wasmBuffer = readFileSync(wasmPath);
  initSync({ module: wasmBuffer });
});

// ---------------------------------------------------------------------------
// 1. State Persistence (save_state / load_state)
// ---------------------------------------------------------------------------
describe('State Persistence (save_state / load_state)', () => {
  it('should save and restore an empty processor', () => {
    const processor = new Sp00kyProcessor();
    const state = processor.save_state();

    const restored = new Sp00kyProcessor();
    restored.load_state(state);

    // Restored processor should work — register a view and get empty results
    const config = createViewConfig('persist-empty', 'SELECT * FROM user');
    const result = restored.register_view(config) as WasmViewUpdate;
    expect(result.result_data).toHaveLength(0);
  });

  it('should save and restore processor with data and views, producing same hash', () => {
    const processor = new Sp00kyProcessor();

    // Ingest data and register view
    const user1 = makeUserRecord('alice', 'alice@test.com');
    const user2 = makeUserRecord('bob', 'bob@test.com');
    processor.ingest('user', 'CREATE', user1.id, user1.record);
    processor.ingest('user', 'CREATE', user2.id, user2.record);

    const config = createViewConfig('persist-data', 'SELECT * FROM user');
    const originalResult = processor.register_view(config) as WasmViewUpdate;
    const originalHash = originalResult.result_hash;
    const originalIds = originalResult.result_data.map((i) => i[0]).sort();

    // Save and restore
    const state = processor.save_state();
    const restored = new Sp00kyProcessor();
    restored.load_state(state);

    // Re-register the same view on restored processor
    const restoredConfig = createViewConfig('persist-data', 'SELECT * FROM user');
    const restoredResult = restored.register_view(restoredConfig) as WasmViewUpdate;
    const restoredIds = restoredResult.result_data.map((i) => i[0]).sort();

    expect(restoredResult.result_hash).toBe(originalHash);
    expect(restoredIds).toEqual(originalIds);
  });

  it('should continue incremental updates after restore', () => {
    const processor = new Sp00kyProcessor();

    const user1 = makeUserRecord('alice', 'alice@test.com');
    processor.ingest('user', 'CREATE', user1.id, user1.record);

    const config = createViewConfig('persist-incr', 'SELECT * FROM user');
    processor.register_view(config);

    // Save and restore
    const state = processor.save_state();
    const restored = new Sp00kyProcessor();
    restored.load_state(state);

    // Re-register the view on restored processor
    const restoredConfig = createViewConfig('persist-incr', 'SELECT * FROM user');
    const viewResult = restored.register_view(restoredConfig) as WasmViewUpdate;
    const hashBefore = viewResult.result_hash;

    // Ingest a new user on the restored processor
    const user2 = makeUserRecord('bob', 'bob@test.com');
    const updates = restored.ingest(
      'user',
      'CREATE',
      user2.id,
      user2.record
    ) as WasmViewUpdate[];

    const viewUpdate = updates.find((u) => u.query_id === 'persist-incr');
    expect(viewUpdate).toBeDefined();
    expect(viewUpdate!.result_hash).not.toBe(hashBefore);
    expect(viewUpdate!.result_data.map((i) => i[0])).toContain(user1.id);
    expect(viewUpdate!.result_data.map((i) => i[0])).toContain(user2.id);
  });

  it('should return valid JSON from save_state', () => {
    const processor = new Sp00kyProcessor();
    const user = makeUserRecord('alice', 'alice@test.com');
    processor.ingest('user', 'CREATE', user.id, user.record);

    const state = processor.save_state();
    expect(typeof state).toBe('string');
    expect(() => JSON.parse(state)).not.toThrow();

    const parsed = JSON.parse(state);
    expect(typeof parsed).toBe('object');
    expect(parsed).not.toBeNull();
  });

  it('should throw when loading corrupted state', () => {
    const processor = new Sp00kyProcessor();
    expect(() => processor.load_state('not-valid-json')).toThrow();
  });
});

// ---------------------------------------------------------------------------
// 2. View Unregistration (unregister_view)
// ---------------------------------------------------------------------------
describe('View Unregistration (unregister_view)', () => {
  const VIEW_ID = 'unreg-view';
  const SQL = 'SELECT * FROM user';

  it('should stop receiving updates after unregistration', () => {
    const processor = new Sp00kyProcessor();

    // Register view and confirm it works
    const user1 = makeUserRecord('alice', 'alice@test.com');
    processor.ingest('user', 'CREATE', user1.id, user1.record);
    const config = createViewConfig(VIEW_ID, SQL);
    processor.register_view(config);

    // Unregister
    processor.unregister_view(VIEW_ID);

    // Ingest another user — no update for unregistered view
    const user2 = makeUserRecord('bob', 'bob@test.com');
    const updates = processor.ingest(
      'user',
      'CREATE',
      user2.id,
      user2.record
    ) as WasmViewUpdate[];

    const viewUpdate = updates.find((u) => u.query_id === VIEW_ID);
    expect(viewUpdate).toBeUndefined();
  });

  it('should not throw when unregistering non-existent view', () => {
    const processor = new Sp00kyProcessor();
    expect(() => processor.unregister_view('nonexistent-view')).not.toThrow();
  });

  it('should allow re-registration after unregister with data intact', () => {
    const processor = new Sp00kyProcessor();

    // Ingest users
    const user1 = makeUserRecord('alice', 'alice@test.com');
    const user2 = makeUserRecord('bob', 'bob@test.com');
    processor.ingest('user', 'CREATE', user1.id, user1.record);
    processor.ingest('user', 'CREATE', user2.id, user2.record);

    // Register, then unregister
    const config1 = createViewConfig(VIEW_ID, SQL);
    processor.register_view(config1);
    processor.unregister_view(VIEW_ID);

    // Re-register — data is still in the store
    const config2 = createViewConfig(VIEW_ID, SQL);
    const result = processor.register_view(config2) as WasmViewUpdate;

    const ids = result.result_data.map((i) => i[0]);
    expect(ids).toContain(user1.id);
    expect(ids).toContain(user2.id);
  });
});

// ---------------------------------------------------------------------------
// 3. UPDATE Operation
// ---------------------------------------------------------------------------
describe('UPDATE Operation', () => {
  const VIEW_ID = 'update-view';
  const SQL = 'SELECT * FROM user';

  it('should emit content update delta without changing membership', () => {
    const processor = new Sp00kyProcessor();

    // Ingest and register
    const user = makeUserRecord('alice', 'alice@test.com');
    processor.ingest('user', 'CREATE', user.id, user.record);
    const config = createViewConfig(VIEW_ID, SQL);
    const initial = processor.register_view(config) as WasmViewUpdate;
    expect(initial.result_data.map((i) => i[0])).toContain(user.id);

    // UPDATE the user
    const updatedRecord = { ...user.record, username: 'alice_updated' };
    const updates = processor.ingest(
      'user',
      'UPDATE',
      user.id,
      updatedRecord
    ) as WasmViewUpdate[];

    const viewUpdate = updates.find((u) => u.query_id === VIEW_ID);
    if (viewUpdate) {
      // Record still in view (membership unchanged)
      expect(viewUpdate.result_data.map((i) => i[0])).toContain(user.id);
      // Should be in updates, not additions
      expect(viewUpdate.delta.additions.map((a) => a[0])).not.toContain(user.id);
      expect(viewUpdate.delta.removals).not.toContain(user.id);
    }
  });

  it('should not emit update for record not in any view', () => {
    const processor = new Sp00kyProcessor();

    // Register user view, but ingest/update an author
    const config = createViewConfig(VIEW_ID, SQL);
    processor.register_view(config);

    const author = makeAuthorRecord('Alice');
    processor.ingest('author', 'CREATE', author.id, author.record);
    const updates = processor.ingest(
      'author',
      'UPDATE',
      author.id,
      { ...author.record, name: 'Bob' }
    ) as WasmViewUpdate[];

    const viewUpdate = updates.find((u) => u.query_id === VIEW_ID);
    expect(viewUpdate).toBeUndefined();
  });

  it('should allow UPDATE followed by DELETE', () => {
    const processor = new Sp00kyProcessor();

    const user = makeUserRecord('alice', 'alice@test.com');
    processor.ingest('user', 'CREATE', user.id, user.record);

    const config = createViewConfig(VIEW_ID, SQL);
    processor.register_view(config);

    // UPDATE
    processor.ingest('user', 'UPDATE', user.id, {
      ...user.record,
      username: 'updated',
    });

    // DELETE
    const deleteUpdates = processor.ingest(
      'user',
      'DELETE',
      user.id,
      {}
    ) as WasmViewUpdate[];

    const viewUpdate = deleteUpdates.find((u) => u.query_id === VIEW_ID);
    expect(viewUpdate).toBeDefined();
    expect(viewUpdate!.result_data.map((i) => i[0])).not.toContain(user.id);
    expect(viewUpdate!.delta.removals).toContain(user.id);
  });
});

// ---------------------------------------------------------------------------
// 4. WHERE Clause Filtering
// ---------------------------------------------------------------------------
describe('WHERE Clause Filtering', () => {
  describe('Equality (=)', () => {
    it('should return only matching records', () => {
      const processor = new Sp00kyProcessor();

      const p1 = makeProductRecord('Phone', 500, 'electronics');
      const p2 = makeProductRecord('Shirt', 30, 'clothing');
      const p3 = makeProductRecord('Laptop', 1200, 'electronics');
      processor.ingest('product', 'CREATE', p1.id, p1.record);
      processor.ingest('product', 'CREATE', p2.id, p2.record);
      processor.ingest('product', 'CREATE', p3.id, p3.record);

      const config = createViewConfig(
        'where-eq',
        "SELECT * FROM product WHERE category = 'electronics'"
      );
      const result = processor.register_view(config) as WasmViewUpdate;
      const ids = result.result_data.map((i) => i[0]);

      expect(ids).toContain(p1.id);
      expect(ids).toContain(p3.id);
      expect(ids).not.toContain(p2.id);
      expect(result.result_data).toHaveLength(2);
    });
  });

  describe('Inequality (!=)', () => {
    it('should return only non-matching records', () => {
      const processor = new Sp00kyProcessor();

      const p1 = makeProductRecord('Phone', 500, 'electronics');
      const p2 = makeProductRecord('Shirt', 30, 'clothing');
      const p3 = makeProductRecord('Apple', 2, 'food');
      processor.ingest('product', 'CREATE', p1.id, p1.record);
      processor.ingest('product', 'CREATE', p2.id, p2.record);
      processor.ingest('product', 'CREATE', p3.id, p3.record);

      const config = createViewConfig(
        'where-neq',
        "SELECT * FROM product WHERE category != 'electronics'"
      );
      const result = processor.register_view(config) as WasmViewUpdate;
      const ids = result.result_data.map((i) => i[0]);

      expect(ids).toContain(p2.id);
      expect(ids).toContain(p3.id);
      expect(ids).not.toContain(p1.id);
    });
  });

  describe('Greater than (>)', () => {
    it('should return only records with value > threshold', () => {
      const processor = new Sp00kyProcessor();

      const p1 = makeProductRecord('Cheap', 30, 'misc');
      const p2 = makeProductRecord('Mid', 50, 'misc');
      const p3 = makeProductRecord('Pricey', 100, 'misc');
      processor.ingest('product', 'CREATE', p1.id, p1.record);
      processor.ingest('product', 'CREATE', p2.id, p2.record);
      processor.ingest('product', 'CREATE', p3.id, p3.record);

      const config = createViewConfig(
        'where-gt',
        'SELECT * FROM product WHERE price > 50'
      );
      const result = processor.register_view(config) as WasmViewUpdate;
      const ids = result.result_data.map((i) => i[0]);

      expect(ids).toContain(p3.id);
      expect(ids).not.toContain(p1.id);
      expect(ids).not.toContain(p2.id);
    });
  });

  describe('Greater than or equal (>=)', () => {
    it('should return records with value >= threshold', () => {
      const processor = new Sp00kyProcessor();

      const p1 = makeProductRecord('Cheap', 30, 'misc');
      const p2 = makeProductRecord('Mid', 50, 'misc');
      const p3 = makeProductRecord('Pricey', 100, 'misc');
      processor.ingest('product', 'CREATE', p1.id, p1.record);
      processor.ingest('product', 'CREATE', p2.id, p2.record);
      processor.ingest('product', 'CREATE', p3.id, p3.record);

      const config = createViewConfig(
        'where-gte',
        'SELECT * FROM product WHERE price >= 50'
      );
      const result = processor.register_view(config) as WasmViewUpdate;
      const ids = result.result_data.map((i) => i[0]);

      expect(ids).toContain(p2.id);
      expect(ids).toContain(p3.id);
      expect(ids).not.toContain(p1.id);
    });
  });

  describe('Less than (<)', () => {
    it('should return only records with value < threshold', () => {
      const processor = new Sp00kyProcessor();

      const p1 = makeProductRecord('Cheap', 30, 'misc');
      const p2 = makeProductRecord('Mid', 50, 'misc');
      const p3 = makeProductRecord('Pricey', 100, 'misc');
      processor.ingest('product', 'CREATE', p1.id, p1.record);
      processor.ingest('product', 'CREATE', p2.id, p2.record);
      processor.ingest('product', 'CREATE', p3.id, p3.record);

      const config = createViewConfig(
        'where-lt',
        'SELECT * FROM product WHERE price < 50'
      );
      const result = processor.register_view(config) as WasmViewUpdate;
      const ids = result.result_data.map((i) => i[0]);

      expect(ids).toContain(p1.id);
      expect(ids).not.toContain(p2.id);
      expect(ids).not.toContain(p3.id);
    });
  });

  describe('Less than or equal (<=)', () => {
    it('should return records with value <= threshold', () => {
      const processor = new Sp00kyProcessor();

      const p1 = makeProductRecord('Cheap', 30, 'misc');
      const p2 = makeProductRecord('Mid', 50, 'misc');
      const p3 = makeProductRecord('Pricey', 100, 'misc');
      processor.ingest('product', 'CREATE', p1.id, p1.record);
      processor.ingest('product', 'CREATE', p2.id, p2.record);
      processor.ingest('product', 'CREATE', p3.id, p3.record);

      const config = createViewConfig(
        'where-lte',
        'SELECT * FROM product WHERE price <= 50'
      );
      const result = processor.register_view(config) as WasmViewUpdate;
      const ids = result.result_data.map((i) => i[0]);

      expect(ids).toContain(p1.id);
      expect(ids).toContain(p2.id);
      expect(ids).not.toContain(p3.id);
    });
  });

  describe('Incremental filtering', () => {
    it('should only add matching records incrementally', () => {
      const processor = new Sp00kyProcessor();

      const config = createViewConfig(
        'where-incr',
        "SELECT * FROM product WHERE category = 'electronics'"
      );
      const initial = processor.register_view(config) as WasmViewUpdate;
      expect(initial.result_data).toHaveLength(0);

      // Ingest non-matching record
      const clothing = makeProductRecord('Shirt', 30, 'clothing');
      const updates1 = processor.ingest(
        'product',
        'CREATE',
        clothing.id,
        clothing.record
      ) as WasmViewUpdate[];
      const viewUpdate1 = updates1.find((u) => u.query_id === 'where-incr');
      // Non-matching record should not appear in view
      if (viewUpdate1) {
        expect(viewUpdate1.result_data.map((i) => i[0])).not.toContain(clothing.id);
      }

      // Ingest matching record
      const phone = makeProductRecord('Phone', 500, 'electronics');
      const updates2 = processor.ingest(
        'product',
        'CREATE',
        phone.id,
        phone.record
      ) as WasmViewUpdate[];
      const viewUpdate2 = updates2.find((u) => u.query_id === 'where-incr');
      expect(viewUpdate2).toBeDefined();
      expect(viewUpdate2!.result_data.map((i) => i[0])).toContain(phone.id);
      expect(viewUpdate2!.result_data).toHaveLength(1);
    });
  });

  describe('Deletion from unfiltered view removes record', () => {
    it('should remove record from non-filtered view on delete', () => {
      const processor = new Sp00kyProcessor();

      const p1 = makeProductRecord('Phone', 500, 'electronics');
      const p2 = makeProductRecord('Laptop', 1200, 'electronics');
      processor.ingest('product', 'CREATE', p1.id, p1.record);
      processor.ingest('product', 'CREATE', p2.id, p2.record);

      const config = createViewConfig('where-del', 'SELECT * FROM product');
      const initial = processor.register_view(config) as WasmViewUpdate;
      expect(initial.result_data).toHaveLength(2);

      // Delete one
      const deleteUpdates = processor.ingest(
        'product',
        'DELETE',
        p1.id,
        {}
      ) as WasmViewUpdate[];

      const viewUpdate = deleteUpdates.find((u) => u.query_id === 'where-del');
      expect(viewUpdate).toBeDefined();
      expect(viewUpdate!.result_data.map((i) => i[0])).not.toContain(p1.id);
      expect(viewUpdate!.result_data.map((i) => i[0])).toContain(p2.id);
      expect(viewUpdate!.delta.removals).toContain(p1.id);
    });
  });
});

// ---------------------------------------------------------------------------
// 5. ORDER BY and LIMIT
// ---------------------------------------------------------------------------
describe('ORDER BY and LIMIT', () => {
  it('should return at most LIMIT N records', () => {
    const processor = new Sp00kyProcessor();

    for (let i = 0; i < 5; i++) {
      const u = makeUserRecord(`user${i}`, `user${i}@test.com`);
      processor.ingest('user', 'CREATE', u.id, u.record);
    }

    const config = createViewConfig('limit-only', 'SELECT * FROM user LIMIT 2');
    const result = processor.register_view(config) as WasmViewUpdate;
    expect(result.result_data).toHaveLength(2);
  });

  it('should return cheapest products with ORDER BY price ASC LIMIT 2', () => {
    const processor = new Sp00kyProcessor();

    const products = [
      makeProductRecord('Expensive', 100, 'misc'),
      makeProductRecord('Cheap', 10, 'misc'),
      makeProductRecord('Mid', 50, 'misc'),
      makeProductRecord('Budget', 20, 'misc'),
    ];
    for (const p of products) {
      processor.ingest('product', 'CREATE', p.id, p.record);
    }

    const config = createViewConfig(
      'order-asc',
      'SELECT * FROM product ORDER BY price ASC LIMIT 2'
    );
    const result = processor.register_view(config) as WasmViewUpdate;
    const ids = result.result_data.map((i) => i[0]);

    // Should contain the 2 cheapest (price=10 and price=20)
    expect(result.result_data).toHaveLength(2);
    expect(ids).toContain(products[1].id); // Cheap (10)
    expect(ids).toContain(products[3].id); // Budget (20)
  });

  it('should return most expensive products with ORDER BY price DESC LIMIT 2', () => {
    const processor = new Sp00kyProcessor();

    const products = [
      makeProductRecord('Expensive', 100, 'misc'),
      makeProductRecord('Cheap', 10, 'misc'),
      makeProductRecord('Mid', 50, 'misc'),
      makeProductRecord('Budget', 20, 'misc'),
    ];
    for (const p of products) {
      processor.ingest('product', 'CREATE', p.id, p.record);
    }

    const config = createViewConfig(
      'order-desc',
      'SELECT * FROM product ORDER BY price DESC LIMIT 2'
    );
    const result = processor.register_view(config) as WasmViewUpdate;
    const ids = result.result_data.map((i) => i[0]);

    // Should contain the 2 most expensive (price=100 and price=50)
    expect(result.result_data).toHaveLength(2);
    expect(ids).toContain(products[0].id); // Expensive (100)
    expect(ids).toContain(products[2].id); // Mid (50)
  });

  it('should displace records when new higher-ranked record is ingested', () => {
    const processor = new Sp00kyProcessor();

    // Start with two cheap products
    const p1 = makeProductRecord('Cheap', 10, 'misc');
    const p2 = makeProductRecord('Budget', 20, 'misc');
    processor.ingest('product', 'CREATE', p1.id, p1.record);
    processor.ingest('product', 'CREATE', p2.id, p2.record);

    const config = createViewConfig(
      'order-displace',
      'SELECT * FROM product ORDER BY price DESC LIMIT 2'
    );
    const initial = processor.register_view(config) as WasmViewUpdate;
    expect(initial.result_data).toHaveLength(2);

    // Ingest an expensive product that should displace the cheapest
    const p3 = makeProductRecord('Premium', 100, 'misc');
    const updates = processor.ingest(
      'product',
      'CREATE',
      p3.id,
      p3.record
    ) as WasmViewUpdate[];

    const viewUpdate = updates.find((u) => u.query_id === 'order-displace');
    expect(viewUpdate).toBeDefined();
    const ids = viewUpdate!.result_data.map((i) => i[0]);

    // Premium (100) should be in, Cheap (10) should be displaced
    expect(ids).toContain(p3.id);
    expect(ids).not.toContain(p1.id);
  });
});

// ---------------------------------------------------------------------------
// 6. Multiple Concurrent Views
// ---------------------------------------------------------------------------
describe('Multiple Concurrent Views', () => {
  it('should deliver updates to correct view when two views filter same table', () => {
    const processor = new Sp00kyProcessor();

    const config1 = createViewConfig(
      'multi-electronics',
      "SELECT * FROM product WHERE category = 'electronics'"
    );
    const config2 = createViewConfig(
      'multi-clothing',
      "SELECT * FROM product WHERE category = 'clothing'"
    );
    processor.register_view(config1);
    processor.register_view(config2);

    // Ingest electronics product
    const phone = makeProductRecord('Phone', 500, 'electronics');
    const updates = processor.ingest(
      'product',
      'CREATE',
      phone.id,
      phone.record
    ) as WasmViewUpdate[];

    const electronicsUpdate = updates.find(
      (u) => u.query_id === 'multi-electronics'
    );
    const clothingUpdate = updates.find(
      (u) => u.query_id === 'multi-clothing'
    );

    // Electronics view should have the product
    expect(electronicsUpdate).toBeDefined();
    expect(electronicsUpdate!.result_data.map((i) => i[0])).toContain(phone.id);

    // Clothing view should not have it
    if (clothingUpdate) {
      expect(clothingUpdate.result_data.map((i) => i[0])).not.toContain(phone.id);
    }
  });

  it('should only update relevant view when tables differ', () => {
    const processor = new Sp00kyProcessor();

    const userConfig = createViewConfig('multi-user', 'SELECT * FROM user');
    const productConfig = createViewConfig(
      'multi-product',
      'SELECT * FROM product'
    );
    processor.register_view(userConfig);
    processor.register_view(productConfig);

    // Ingest a user
    const user = makeUserRecord('alice', 'alice@test.com');
    const updates = processor.ingest(
      'user',
      'CREATE',
      user.id,
      user.record
    ) as WasmViewUpdate[];

    const userUpdate = updates.find((u) => u.query_id === 'multi-user');
    const productUpdate = updates.find((u) => u.query_id === 'multi-product');

    expect(userUpdate).toBeDefined();
    expect(userUpdate!.result_data.map((i) => i[0])).toContain(user.id);

    // Product view should not be affected
    expect(productUpdate).toBeUndefined();
  });

  it('should not affect other views when one is unregistered', () => {
    const processor = new Sp00kyProcessor();

    const config1 = createViewConfig('multi-a', 'SELECT * FROM user');
    const config2 = createViewConfig('multi-b', 'SELECT * FROM user');
    processor.register_view(config1);
    processor.register_view(config2);

    const user1 = makeUserRecord('alice', 'alice@test.com');
    processor.ingest('user', 'CREATE', user1.id, user1.record);

    // Unregister view-a
    processor.unregister_view('multi-a');

    // Ingest another user
    const user2 = makeUserRecord('bob', 'bob@test.com');
    const updates = processor.ingest(
      'user',
      'CREATE',
      user2.id,
      user2.record
    ) as WasmViewUpdate[];

    const updateA = updates.find((u) => u.query_id === 'multi-a');
    const updateB = updates.find((u) => u.query_id === 'multi-b');

    expect(updateA).toBeUndefined();
    expect(updateB).toBeDefined();
    expect(updateB!.result_data.map((i) => i[0])).toContain(user1.id);
    expect(updateB!.result_data.map((i) => i[0])).toContain(user2.id);
  });
});

// ---------------------------------------------------------------------------
// 7. Parameterized Queries ($param)
// ---------------------------------------------------------------------------
describe('Parameterized Queries ($param)', () => {
  it('should filter records using $param from view config', () => {
    const processor = new Sp00kyProcessor();

    const alice = makeUserRecord('alice', 'alice@test.com');
    const bob = makeUserRecord('bob', 'bob@test.com');
    processor.ingest('user', 'CREATE', alice.id, alice.record);
    processor.ingest('user', 'CREATE', bob.id, bob.record);

    const config = createViewConfig(
      'param-filter',
      'SELECT * FROM user WHERE username = $target_user',
      { target_user: 'alice' }
    );
    const result = processor.register_view(config) as WasmViewUpdate;
    const ids = result.result_data.map((i) => i[0]);

    expect(ids).toContain(alice.id);
    expect(ids).not.toContain(bob.id);
  });

  it('should produce different results for different param values', () => {
    const processor = new Sp00kyProcessor();

    const alice = makeUserRecord('alice', 'alice@test.com');
    const bob = makeUserRecord('bob', 'bob@test.com');
    processor.ingest('user', 'CREATE', alice.id, alice.record);
    processor.ingest('user', 'CREATE', bob.id, bob.record);

    const config1 = createViewConfig(
      'param-alice',
      'SELECT * FROM user WHERE username = $target_user',
      { target_user: 'alice' }
    );
    const config2 = createViewConfig(
      'param-bob',
      'SELECT * FROM user WHERE username = $target_user',
      { target_user: 'bob' }
    );

    const result1 = processor.register_view(config1) as WasmViewUpdate;
    const result2 = processor.register_view(config2) as WasmViewUpdate;

    expect(result1.result_data.map((i) => i[0])).toContain(alice.id);
    expect(result1.result_data.map((i) => i[0])).not.toContain(bob.id);

    expect(result2.result_data.map((i) => i[0])).toContain(bob.id);
    expect(result2.result_data.map((i) => i[0])).not.toContain(alice.id);
  });
});

// ---------------------------------------------------------------------------
// 8. Empty and Invalid Records
// ---------------------------------------------------------------------------
describe('Edge Cases: Empty and Invalid Records', () => {
  it('should handle ingestion of empty object without crashing', () => {
    const processor = new Sp00kyProcessor();

    const config = createViewConfig('empty-rec', 'SELECT * FROM user');
    processor.register_view(config);

    // Ingest empty record — the id param is used as fallback
    expect(() =>
      processor.ingest('user', 'CREATE', 'user:empty_one', {})
    ).not.toThrow();

    const updates = processor.ingest(
      'user',
      'CREATE',
      'user:empty_two',
      {}
    ) as WasmViewUpdate[];

    // Should not crash; may or may not appear in view
    expect(updates).toBeDefined();
  });

  it('should exclude records with missing fields from filtered view', () => {
    const processor = new Sp00kyProcessor();

    const config = createViewConfig(
      'missing-field',
      'SELECT * FROM user WHERE age > 18'
    );
    processor.register_view(config);

    // User without age field
    const user = makeUserRecord('alice', 'alice@test.com');
    const updates = processor.ingest(
      'user',
      'CREATE',
      user.id,
      user.record
    ) as WasmViewUpdate[];

    const viewUpdate = updates.find((u) => u.query_id === 'missing-field');
    if (viewUpdate) {
      // Record should not be in view since it lacks 'age' field
      expect(viewUpdate.result_data.map((i) => i[0])).not.toContain(user.id);
    }

    // User with age field should appear
    const userWithAge = makeUserRecordExtended('bob', 'bob@test.com', 25, 1);
    const updates2 = processor.ingest(
      'user',
      'CREATE',
      userWithAge.id,
      userWithAge.record
    ) as WasmViewUpdate[];

    const viewUpdate2 = updates2.find((u) => u.query_id === 'missing-field');
    expect(viewUpdate2).toBeDefined();
    expect(viewUpdate2!.result_data.map((i) => i[0])).toContain(userWithAge.id);
  });

  it('should prioritize id field from record over id param', () => {
    const processor = new Sp00kyProcessor();

    // Record has id = 'user:from_record', but param id = 'user:from_param'
    const record = {
      id: 'user:from_record',
      username: 'test',
      type: 'user',
    };
    processor.ingest('user', 'CREATE', 'user:from_param', record);

    const config = createViewConfig('id-priority', 'SELECT * FROM user');
    const result = processor.register_view(config) as WasmViewUpdate;
    const ids = result.result_data.map((i) => i[0]);

    // The record's own id should be used
    expect(ids).toContain('user:from_record');
  });
});

// ---------------------------------------------------------------------------
// 9. Duplicate and Non-Existent Operations
// ---------------------------------------------------------------------------
describe('Edge Cases: Duplicate and Non-Existent Operations', () => {
  it('should handle creating the same record ID twice without crashing', () => {
    const processor = new Sp00kyProcessor();

    const config = createViewConfig('dup-create', 'SELECT * FROM user');
    processor.register_view(config);

    const record1 = {
      id: 'user:dup',
      username: 'alice',
      email: 'alice@test.com',
      type: 'user',
    };
    const record2 = {
      id: 'user:dup',
      username: 'bob',
      email: 'bob@test.com',
      type: 'user',
    };

    expect(() =>
      processor.ingest('user', 'CREATE', 'user:dup', record1)
    ).not.toThrow();
    expect(() =>
      processor.ingest('user', 'CREATE', 'user:dup', record2)
    ).not.toThrow();

    // Record should still be in the view
    const user3 = makeUserRecord('charlie', 'charlie@test.com');
    const updates = processor.ingest(
      'user',
      'CREATE',
      user3.id,
      user3.record
    ) as WasmViewUpdate[];
    const viewUpdate = updates.find((u) => u.query_id === 'dup-create');
    if (viewUpdate) {
      expect(viewUpdate.result_data.map((i) => i[0])).toContain('user:dup');
    }
  });

  it('should handle deleting a non-existent record without crashing', () => {
    const processor = new Sp00kyProcessor();

    const config = createViewConfig('del-nonexist', 'SELECT * FROM user');
    processor.register_view(config);

    expect(() =>
      processor.ingest('user', 'DELETE', 'user:nonexistent', {})
    ).not.toThrow();
  });

  it('should not add UPDATE-only record to view (weight 0 = no membership)', () => {
    const processor = new Sp00kyProcessor();

    const config = createViewConfig('update-ghost', 'SELECT * FROM user');
    const initial = processor.register_view(config) as WasmViewUpdate;
    expect(initial.result_data).toHaveLength(0);

    // UPDATE without prior CREATE
    const record = {
      id: 'user:ghost',
      username: 'ghost',
      type: 'user',
    };
    const updates = processor.ingest(
      'user',
      'UPDATE',
      'user:ghost',
      record
    ) as WasmViewUpdate[];

    const viewUpdate = updates.find((u) => u.query_id === 'update-ghost');
    if (viewUpdate) {
      // Record should NOT be in the view (UPDATE has weight 0)
      expect(viewUpdate.result_data.map((i) => i[0])).not.toContain(
        'user:ghost'
      );
    }
  });

  it('should allow delete followed by re-create of same ID', () => {
    const processor = new Sp00kyProcessor();

    const record = {
      id: 'user:phoenix',
      username: 'phoenix',
      email: 'phoenix@test.com',
      type: 'user',
    };
    processor.ingest('user', 'CREATE', 'user:phoenix', record);

    const config = createViewConfig('del-recreate', 'SELECT * FROM user');
    const initial = processor.register_view(config) as WasmViewUpdate;
    expect(initial.result_data.map((i) => i[0])).toContain('user:phoenix');

    // Delete
    const deleteUpdates = processor.ingest(
      'user',
      'DELETE',
      'user:phoenix',
      {}
    ) as WasmViewUpdate[];
    const delUpdate = deleteUpdates.find((u) => u.query_id === 'del-recreate');
    expect(delUpdate).toBeDefined();
    expect(delUpdate!.result_data.map((i) => i[0])).not.toContain(
      'user:phoenix'
    );

    // Re-create
    const createUpdates = processor.ingest(
      'user',
      'CREATE',
      'user:phoenix',
      record
    ) as WasmViewUpdate[];
    const createUpdate = createUpdates.find(
      (u) => u.query_id === 'del-recreate'
    );
    expect(createUpdate).toBeDefined();
    expect(createUpdate!.result_data.map((i) => i[0])).toContain(
      'user:phoenix'
    );
  });
});

// ---------------------------------------------------------------------------
// 10. View Registration Edge Cases
// ---------------------------------------------------------------------------
describe('Edge Cases: View Registration', () => {
  it('should overwrite view when registering same ID twice', () => {
    const processor = new Sp00kyProcessor();

    // Register a user view
    const config1 = createViewConfig('overwrite-view', 'SELECT * FROM user');
    processor.register_view(config1);

    const user = makeUserRecord('alice', 'alice@test.com');
    processor.ingest('user', 'CREATE', user.id, user.record);

    // Re-register with product query
    const config2 = createViewConfig(
      'overwrite-view',
      'SELECT * FROM product'
    );
    processor.register_view(config2);

    // Ingest product — should trigger update on overwritten view
    const product = makeProductRecord('Phone', 500, 'electronics');
    const updates = processor.ingest(
      'product',
      'CREATE',
      product.id,
      product.record
    ) as WasmViewUpdate[];

    const viewUpdate = updates.find((u) => u.query_id === 'overwrite-view');
    expect(viewUpdate).toBeDefined();
    expect(viewUpdate!.result_data.map((i) => i[0])).toContain(product.id);
  });

  it('should throw when registering view with invalid SQL', () => {
    const processor = new Sp00kyProcessor();
    const config = createViewConfig('bad-sql', 'NOT VALID SQL AT ALL');
    expect(() => processor.register_view(config)).toThrow();
  });

  it('should throw when registering view with empty SQL', () => {
    const processor = new Sp00kyProcessor();
    const config = createViewConfig('empty-sql', '');
    expect(() => processor.register_view(config)).toThrow();
  });
});

// ---------------------------------------------------------------------------
// 11. Ingest to Unrelated Table
// ---------------------------------------------------------------------------
describe('Ingest to Unrelated Table', () => {
  it('should return empty updates when ingesting to unrelated table', () => {
    const processor = new Sp00kyProcessor();

    const config = createViewConfig('unrelated', 'SELECT * FROM user');
    processor.register_view(config);

    // Ingest a product — user view should not be affected
    const product = makeProductRecord('Phone', 500, 'electronics');
    const updates = processor.ingest(
      'product',
      'CREATE',
      product.id,
      product.record
    ) as WasmViewUpdate[];

    const viewUpdate = updates.find((u) => u.query_id === 'unrelated');
    expect(viewUpdate).toBeUndefined();
  });

  it('should not change view hash when ingesting to unrelated table', () => {
    const processor = new Sp00kyProcessor();

    const user = makeUserRecord('alice', 'alice@test.com');
    processor.ingest('user', 'CREATE', user.id, user.record);

    const config = createViewConfig('unrelated-hash', 'SELECT * FROM user');
    const initial = processor.register_view(config) as WasmViewUpdate;
    const hashBefore = initial.result_hash;

    // Ingest to unrelated table
    const product = makeProductRecord('Phone', 500, 'electronics');
    processor.ingest('product', 'CREATE', product.id, product.record);

    // Ingest another user to check hash is still correct
    const user2 = makeUserRecord('bob', 'bob@test.com');
    const updates = processor.ingest(
      'user',
      'CREATE',
      user2.id,
      user2.record
    ) as WasmViewUpdate[];
    const viewUpdate = updates.find((u) => u.query_id === 'unrelated-hash');
    expect(viewUpdate).toBeDefined();
    // Hash should have changed because of the new user, not because of the product
    expect(viewUpdate!.result_hash).not.toBe(hashBefore);
    expect(viewUpdate!.result_data).toHaveLength(2);
  });
});

// ---------------------------------------------------------------------------
// 12. Large Batch Operations
// ---------------------------------------------------------------------------
describe('Large Batch Operations', () => {
  it('should handle 100 records correctly', () => {
    const processor = new Sp00kyProcessor();

    const users: { id: string; record: Record<string, unknown> }[] = [];
    for (let i = 0; i < 100; i++) {
      const u = makeUserRecord(`user_${i}`, `user${i}@test.com`);
      processor.ingest('user', 'CREATE', u.id, u.record);
      users.push(u);
    }

    const config = createViewConfig('batch-100', 'SELECT * FROM user');
    const result = processor.register_view(config) as WasmViewUpdate;

    expect(result.result_data).toHaveLength(100);
    expect(validateFlatArray(result.result_data)).toBe(true);
  });

  it('should handle 100 inserts then 50 deletes correctly', () => {
    const processor = new Sp00kyProcessor();

    const users: { id: string; record: Record<string, unknown> }[] = [];
    for (let i = 0; i < 100; i++) {
      const u = makeUserRecord(`user_${i}`, `user${i}@test.com`);
      processor.ingest('user', 'CREATE', u.id, u.record);
      users.push(u);
    }

    const config = createViewConfig('batch-del', 'SELECT * FROM user');
    processor.register_view(config);

    // Delete first 50
    for (let i = 0; i < 50; i++) {
      processor.ingest('user', 'DELETE', users[i].id, {});
    }

    // Ingest one more to trigger a fresh update
    const extra = makeUserRecord('extra', 'extra@test.com');
    const updates = processor.ingest(
      'user',
      'CREATE',
      extra.id,
      extra.record
    ) as WasmViewUpdate[];

    const viewUpdate = updates.find((u) => u.query_id === 'batch-del');
    expect(viewUpdate).toBeDefined();
    // 100 - 50 + 1 = 51 records
    expect(viewUpdate!.result_data).toHaveLength(51);

    // Deleted IDs should not be present
    const ids = viewUpdate!.result_data.map((i) => i[0]);
    for (let i = 0; i < 50; i++) {
      expect(ids).not.toContain(users[i].id);
    }
  });

  it('should respect LIMIT with large dataset', () => {
    const processor = new Sp00kyProcessor();

    for (let i = 0; i < 100; i++) {
      const u = makeUserRecord(`user_${i}`, `user${i}@test.com`);
      processor.ingest('user', 'CREATE', u.id, u.record);
    }

    const config = createViewConfig(
      'batch-limit',
      'SELECT * FROM user LIMIT 10'
    );
    const result = processor.register_view(config) as WasmViewUpdate;

    expect(result.result_data).toHaveLength(10);
    expect(validateFlatArray(result.result_data)).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// 13. Version Tracking
// ---------------------------------------------------------------------------
describe('Version Tracking', () => {
  it('should default version to 1 for records without _00_rv', () => {
    const processor = new Sp00kyProcessor();

    const user = makeUserRecord('alice', 'alice@test.com');
    processor.ingest('user', 'CREATE', user.id, user.record);

    const config = createViewConfig('version-default', 'SELECT * FROM user');
    const result = processor.register_view(config) as WasmViewUpdate;

    const entry = result.result_data.find((i) => i[0] === user.id);
    expect(entry).toBeDefined();
    expect(entry![1]).toBe(1); // Default version
  });

  it('should reflect _00_rv field value as version', () => {
    const processor = new Sp00kyProcessor();

    const id = `user:${generateId()}`;
    const record = {
      id,
      username: 'versioned',
      email: 'v@test.com',
      type: 'user',
      _00_rv: 5,
    };
    processor.ingest('user', 'CREATE', id, record);

    const config = createViewConfig('version-custom', 'SELECT * FROM user');
    const result = processor.register_view(config) as WasmViewUpdate;

    const entry = result.result_data.find((i) => i[0] === id);
    expect(entry).toBeDefined();
    expect(entry![1]).toBe(5);
  });
});

// ---------------------------------------------------------------------------
// 14. Delta Structure Validation
// ---------------------------------------------------------------------------
describe('Delta Structure Validation', () => {
  it('should have additions on CREATE', () => {
    const processor = new Sp00kyProcessor();

    const config = createViewConfig('delta-create', 'SELECT * FROM user');
    processor.register_view(config);

    const user = makeUserRecord('alice', 'alice@test.com');
    const updates = processor.ingest(
      'user',
      'CREATE',
      user.id,
      user.record
    ) as WasmViewUpdate[];

    const viewUpdate = updates.find((u) => u.query_id === 'delta-create');
    expect(viewUpdate).toBeDefined();
    expect(viewUpdate!.delta.additions.length).toBeGreaterThan(0);
    expect(viewUpdate!.delta.additions[0][0]).toBe(user.id);
    expect(typeof viewUpdate!.delta.additions[0][1]).toBe('number');
    expect(viewUpdate!.delta.removals).toHaveLength(0);
    expect(viewUpdate!.delta.updates).toHaveLength(0);
  });

  it('should have removals on DELETE', () => {
    const processor = new Sp00kyProcessor();

    const user = makeUserRecord('alice', 'alice@test.com');
    processor.ingest('user', 'CREATE', user.id, user.record);

    const config = createViewConfig('delta-delete', 'SELECT * FROM user');
    processor.register_view(config);

    const updates = processor.ingest(
      'user',
      'DELETE',
      user.id,
      {}
    ) as WasmViewUpdate[];

    const viewUpdate = updates.find((u) => u.query_id === 'delta-delete');
    expect(viewUpdate).toBeDefined();
    expect(viewUpdate!.delta.removals).toContain(user.id);
    expect(viewUpdate!.delta.additions).toHaveLength(0);
  });

  it('should have all records as additions on initial registration', () => {
    const processor = new Sp00kyProcessor();

    const user1 = makeUserRecord('alice', 'alice@test.com');
    const user2 = makeUserRecord('bob', 'bob@test.com');
    const user3 = makeUserRecord('charlie', 'charlie@test.com');
    processor.ingest('user', 'CREATE', user1.id, user1.record);
    processor.ingest('user', 'CREATE', user2.id, user2.record);
    processor.ingest('user', 'CREATE', user3.id, user3.record);

    const config = createViewConfig('delta-initial', 'SELECT * FROM user');
    const result = processor.register_view(config) as WasmViewUpdate;

    expect(result.delta.additions).toHaveLength(3);
    const additionIds = result.delta.additions.map((a) => a[0]);
    expect(additionIds).toContain(user1.id);
    expect(additionIds).toContain(user2.id);
    expect(additionIds).toContain(user3.id);
    expect(result.delta.removals).toHaveLength(0);
    expect(result.delta.updates).toHaveLength(0);
  });

  it('should have updates (not additions) on UPDATE of existing record', () => {
    const processor = new Sp00kyProcessor();

    const user = makeUserRecord('alice', 'alice@test.com');
    processor.ingest('user', 'CREATE', user.id, user.record);

    const config = createViewConfig('delta-update', 'SELECT * FROM user');
    processor.register_view(config);

    const updates = processor.ingest('user', 'UPDATE', user.id, {
      ...user.record,
      username: 'alice_updated',
    }) as WasmViewUpdate[];

    const viewUpdate = updates.find((u) => u.query_id === 'delta-update');
    if (viewUpdate) {
      // Should be in updates, not in additions
      expect(viewUpdate.delta.additions).toHaveLength(0);
      expect(viewUpdate.delta.removals).toHaveLength(0);
      const updateIds = viewUpdate.delta.updates.map((u) => u[0]);
      expect(updateIds).toContain(user.id);
    }
  });
});
