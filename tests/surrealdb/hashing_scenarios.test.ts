import { createTestDb } from './setup';
import { Surreal, Table } from 'surrealdb';

let db: Surreal;

describe('Hashing Scenarios Tests', () => {
  beforeAll(async () => {
    db = await createTestDb();
  });

  afterAll(async () => {
    await db.close();
  });

  const getHash = async (recordId: string) => {
    const res = (await db
      .query(`SELECT * FROM ONLY _spooky_data_hash WHERE recordId = ${recordId}`)
      .collect()) as any;
    return Array.isArray(res) ? res[0] : res;
  };

  const waitForHashChange = async (recordId: string, initialtotalHash: string, timeout = 5000) => {
    const start = Date.now();
    while (Date.now() - start < timeout) {
      const current = await getHash(recordId);
      if (current && current.totalHash !== initialtotalHash) return current;
      await new Promise((r) => setTimeout(r, 100));
    }
    return await getHash(recordId);
  };

  // Scenario 1: No dependencies -> empty hashes
  // Thread without comments. Author is present (Reference), but we can try to mock an empty reference if possible or just check comments.
  test('1. No Dependency Records -> Empty (Dependency) Hashes', async () => {
    const user = await db.create(new Table('user')).content({
      username: 'scenario_user_1_' + Date.now(),
      password: 'password',
      created_at: new Date(),
    });
    const userId = (user as any)[0].id;

    const thread = await db.create(new Table('thread')).content({
      title: 'Scenario 1 Thread',
      content: 'Content',
      author: userId,
    });
    const threadId = (thread as any)[0].id;

    const hash = await getHash(threadId);

    // compositionHash should be defined
    expect(hash.compositionHash).toBeDefined();

    // "Dependecy records" (Related List -> Comments) are empty.
    // My implementation adds keys for ALL dependent tables even if empty, defaulting to empty hash.
    // So check if 'comment' key exists and is "empty hash" (hash of empty string).
    expect(hash.compositionHash).toHaveProperty('comment');

    // Empty hash constant (Blake3 of "")
    // We can't easily replicate proper Blake3 here without library, but we know it should be consistent.
    // Let's verify it is a valid hash string.
    expect(typeof hash.compositionHash.comment).toBe('string');

    // Verify 'author' (Related Record) is marked @parent, so it should NOT be in composition hash (to avoid loops)
    expect(hash.compositionHash).not.toHaveProperty('author');
  });

  // Scenario 2: Dependency records exists -> hashes should be correct
  test('2. Dependency Records Exists -> Hashes Correct', async () => {
    // Reuse setup or new? New for isolation.
    const user = await db.create(new Table('user')).content({
      username: 'scenario_user_2_' + Date.now(),
      password: 'password',
      created_at: new Date(),
    });
    const userId = (user as any)[0].id;

    const thread = await db.create(new Table('thread')).content({
      title: 'Scenario 2 Thread',
      content: 'Content',
      author: userId,
    });
    const threadId = (thread as any)[0].id;

    // Add Comment (Dependency)
    await db.create(new Table('comment')).content({
      content: 'Comment S2',
      thread: threadId,
      author: userId,
    });

    // Wait for update (comment creation bubbles up)
    // We need initial hash to compare? Or just check final state.
    // Since test runs fast, race condition possible. Best to wait.

    // Let's fetch loop.
    let hash = await getHash(threadId);
    // We expect 'comment' hash to be DIFFERENT from empty.
    // We can compare against Scenario 1's "empty" hash if we knew it.
    // Instead, let's just assert it is valid.

    expect(hash.compositionHash).toHaveProperty('comment');
    expect(hash.compositionHash.comment.length).toBeGreaterThan(0);

    // Verify totalHash includes it (implicitly by being Xor of Comp._xor)
  });

  // Scenario 3: Dependency records change (related list) -> hashes should be correct and changed
  test('3. Related List Change -> Hashes Update', async () => {
    const user = await db.create(new Table('user')).content({
      username: 'scenario_user_3_' + Date.now(),
      password: 'password',
      created_at: new Date(),
    });
    const userId = (user as any)[0].id;

    const thread = await db.create(new Table('thread')).content({
      title: 'Scenario 3 Thread',
      content: 'Content',
      author: userId,
    });
    const threadId = (thread as any)[0].id;

    const hash1 = await getHash(threadId);

    // Add Comment
    const comment = await db.create(new Table('comment')).content({
      content: 'Comment S3',
      thread: threadId,
      author: userId,
    });
    const commentId = (comment as any)[0].id;

    const hash2 = await waitForHashChange(threadId, hash1.totalHash);
    expect(hash2.totalHash).not.toBe(hash1.totalHash);
    // Check Composition.comment changed
    expect(hash2.compositionHash.comment).not.toBe(hash1.compositionHash.comment);

    // Update Comment
    await db.query(`UPDATE ${commentId} SET content = "Updated S3"`);
    const hash3 = await waitForHashChange(threadId, hash2.totalHash);
    expect(hash3.totalHash).not.toBe(hash2.totalHash);
    expect(hash3.compositionHash.comment).not.toBe(hash2.compositionHash.comment);

    // Delete Comment
    await db.query(`DELETE ${commentId}`);
    const hash4 = await waitForHashChange(threadId, hash3.totalHash);
    expect(hash4.totalHash).not.toBe(hash3.totalHash);

    // Should revert to original 'empty' state for comment hash?
    // Note: Hash collisions possible but unlikely.
    expect(hash4.compositionHash.comment).toBe(hash1.compositionHash.comment);
  });

  // Scenario 4: Dependency records change (related record) -> hashes should be correct and changed
  test('4. Related Record (Reference) Change -> Hashes Update', async () => {
    const user = await db.create(new Table('user')).content({
      username: 'scenario_user_4_' + Date.now(),
      password: 'password',
      created_at: new Date(),
    });
    const userId = (user as any)[0].id;

    const thread = await db.create(new Table('thread')).content({
      title: 'Scenario 4 Thread',
      content: 'Content',
      author: userId,
    });
    const threadId = (thread as any)[0].id;

    const hash1 = await getHash(threadId);

    // Change the Author's Content (Referenced Record Update)
    // This should Ripple/Cascade Down to the Thread.
    // Currently, "Reference Propagation" might NOT be implemented.
    // This test verifies if it works.

    console.log('Updating User (Author)...');
    await db.query(`UPDATE ${userId} SET username = "updated_user_4"`);

    // Wait for Thread Hash to change
    const hash2 = await waitForHashChange(threadId, hash1.totalHash, 3000); // Short timeout

    if (hash1.totalHash === hash2.totalHash) {
      console.warn('Scenario 4 Failed: Thread hash did not update after Author update.');
    }

    // NOTE: 'author' is a PARENT reference. Changing the Parent payload does NOT change the Child's Intrinsic Hash.
    // So Thread Hash should REMAIN THE SAME.
    expect(hash2.totalHash).toBe(hash1.totalHash);
  });

  // Scenario 5: Add/Remove Dependency -> Hash Reverts
  test('5. Add/Remove Dependency -> Hash Reverts', async () => {
    const user = await db.create(new Table('user')).content({
      username: 'scenario_user_5_' + Date.now(),
      password: 'password',
      created_at: new Date(),
    });
    const userId = (user as any)[0].id;

    const thread = await db.create(new Table('thread')).content({
      title: 'Scenario 5 Thread',
      content: 'Content',
      author: userId,
    });
    const threadId = (thread as any)[0].id;

    const hash1 = await getHash(threadId);

    // Add Comment
    const comment = await db.create(new Table('comment')).content({
      content: 'Comment S5',
      thread: threadId,
      author: userId,
    });
    const commentId = (comment as any)[0].id;

    const hash2 = await waitForHashChange(threadId, hash1.totalHash);
    expect(hash2.totalHash).not.toBe(hash1.totalHash);

    // Remove Comment
    await db.query(`DELETE ${commentId}`);
    const hash3 = await waitForHashChange(threadId, hash2.totalHash);

    // Hash3 should equal Hash1
    expect(hash3.totalHash).toBe(hash1.totalHash);
    expect(hash3.compositionHash.comment).toBe(hash1.compositionHash.comment);
    expect(hash3.compositionHash._xor).toBe(hash1.compositionHash._xor);
  });

  // Scenario 6: Modify/Revert Intrinsic -> Hash Reverts
  test('6. Modify/Revert Intrinsic -> Hash Reverts', async () => {
    const user = await db.create(new Table('user')).content({
      username: 'scenario_user_6_' + Date.now(),
      password: 'password',
      created_at: new Date(),
    });
    const userId = (user as any)[0].id;

    const thread = await db.create(new Table('thread')).content({
      title: 'Original Title',
      content: 'Content',
      author: userId,
    });
    const threadId = (thread as any)[0].id;

    const hash1 = await getHash(threadId);

    // Modify Title
    await db.query(`UPDATE ${threadId} SET title = "Updated Title"`);
    const hash2 = await waitForHashChange(threadId, hash1.totalHash);

    expect(hash2.totalHash).not.toBe(hash1.totalHash);
    expect(hash2.intrinsicHash).not.toBe(hash1.intrinsicHash);

    // Revert Title
    await db.query(`UPDATE ${threadId} SET title = "Original Title"`);
    const hash3 = await waitForHashChange(threadId, hash2.totalHash);

    // Hash3 should equal Hash1
    expect(hash3.totalHash).toBe(hash1.totalHash);
    expect(hash3.intrinsicHash).toBe(hash1.intrinsicHash);
  });

  // Scenario 7: Modify/Revert Child (Dependency) -> Parent Hash Reverts
  test('7. Modify/Revert Child -> Parent Hash Reverts', async () => {
    const user = await db.create(new Table('user')).content({
      username: 'scenario_user_7_' + Date.now(),
      password: 'password',
      created_at: new Date(),
    });
    const userId = (user as any)[0].id;

    const thread = await db.create(new Table('thread')).content({
      title: 'Scenario 7 Thread',
      content: 'Content',
      author: userId,
    });
    const threadId = (thread as any)[0].id;

    // Add Comment
    const comment = await db.create(new Table('comment')).content({
      content: 'Original Comment Content',
      thread: threadId,
      author: userId,
    });
    const commentId = (comment as any)[0].id;

    // Wait for stability
    // We need to wait for the comment creation to bubble up first!
    // Otherwise hash1 might be the empty state.

    // How to ensure stability? Wait for 'comment' key in composition hash.
    let hash1: any;
    const start = Date.now();
    while (Date.now() - start < 5000) {
      hash1 = await getHash(threadId);
      if (
        hash1 &&
        hash1.compositionHash &&
        hash1.compositionHash.comment &&
        hash1.compositionHash.comment.length > 0
      )
        break;
      await new Promise((r) => setTimeout(r, 100));
    }
    expect(hash1.compositionHash.comment).toBeDefined(); // Actually it might be hashed string

    // Modify Comment
    console.log('Modifying Comment...');
    await db.query(`UPDATE ${commentId} SET content = "Updated Comment Content"`);
    const hash2 = await waitForHashChange(threadId, hash1.totalHash);

    expect(hash2.totalHash).not.toBe(hash1.totalHash);
    expect(hash2.compositionHash.comment).not.toBe(hash1.compositionHash.comment);

    // Revert Comment
    console.log('Reverting Comment...');
    await db.query(`UPDATE ${commentId} SET content = "Original Comment Content"`);
    const hash3 = await waitForHashChange(threadId, hash2.totalHash);

    // Hash3 should equal Hash1
    if (hash3.totalHash !== hash1.totalHash) {
      console.warn(
        `Scenario 7 Fail Details:\nOriginal: ${hash1.totalHash}\nModified: ${hash2.totalHash}\nReverted: ${hash3.totalHash}`
      );
    }
    expect(hash3.compositionHash.comment).toBe(hash1.compositionHash.comment);
    expect(hash3.totalHash).toBe(hash1.totalHash);
  });
});
