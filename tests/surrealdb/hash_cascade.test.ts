import { createTestDb } from './setup';
import { Surreal, Table } from 'surrealdb';

let db: Surreal;

describe('Hash Cascade Logic', () => {
  beforeAll(async () => {
    db = await createTestDb();
  });

  afterAll(async () => {
    await db.close();
  });

  test('Should execute cascade update step-by-step', async () => {
    // 1. Create User
    const user = await db.create(new Table('user')).content({
      username: 'cascade_user_' + Date.now(),
      password: 'password',
      created_at: new Date(),
    });
    const userId = (user as any)[0].id;
    console.log('User Created:', userId);

    // 2. Create Thread
    const thread = await db.create(new Table('thread')).content({
      title: 'Cascade Thread',
      content: 'Content',
      author: userId,
    });
    const threadId = (thread as any)[0].id;
    console.log('Thread Created:', threadId);

    // 3. Create Comment
    const comment = await db.create(new Table('comment')).content({
      content: 'Cascade Comment',
      thread: threadId,
      author: userId,
    });
    const commentId = (comment as any)[0].id;
    console.log('Comment Created:', commentId);

    // 4. Capture Initial Hash
    const getHash = async (recordId: string) => {
      const res = (await db
        .query(`SELECT * FROM ONLY _spooky_data_hash WHERE recordId = ${recordId}`)
        .collect()) as any;
      return Array.isArray(res) ? res[0] : res;
    };
    const initialHash = await getHash(threadId);
    // const initialUserHash = await getHash(userId);

    console.log('Initial Thread Hash:', initialHash.totalHash);
    // console.log("Initial User Hash:", initialUserHash?.totalHash);

    expect(initialHash.totalHash).toBeDefined();

    // 4. Update Comment -> Triggers Cascade (Bubble)
    // Note: Updating User (Reference) does not change Thread Hash unless schema embeds User.
    // Updating Comment (Child) creates a delta that propagates to Thread (Parent).
    try {
      console.log('Updating Comment...');
      // Use query to control update precisely
      const result = await db
        .query(`UPDATE ${commentId} SET content = "updated_comment_${Date.now()}"`)
        .collect();
      console.log('Comment Update Result:', JSON.stringify(result));
    } catch (e: any) {
      console.error('Comment Update Failed:', e);
      throw e;
    }

    // Wait for cascade
    const waitForHashChange = async (initial: string, timeout = 5000) => {
      const start = Date.now();
      while (Date.now() - start < timeout) {
        const current = await getHash(threadId);
        if (current && current.totalHash !== initial) return current;
        await new Promise((r) => setTimeout(r, 200));
      }
      return await getHash(threadId);
    };

    const finalHash = await waitForHashChange(initialHash.totalHash);

    console.log('Final Thread Hash:', finalHash.totalHash);

    // Assert record exists and has hashes
    // Assert record exists and has hashes
    expect(finalHash.intrinsicHash).toBeDefined();
    // intrinsicHash is now a SCALAR string (XOR of fields)
    expect(typeof finalHash.intrinsicHash).toBe('string');

    // compositionHash is now an OBJECT containing breakdown
    expect(finalHash.compositionHash).toBeDefined();
    expect(typeof finalHash.compositionHash).toBe('object');

    // Verify Intrinsic Fields are NOT in compositionHash.
    // Thread fields: title, content, author. These are in intrinsicHash XOR.
    expect(finalHash.compositionHash).not.toHaveProperty('title');
    expect(finalHash.compositionHash).not.toHaveProperty('content');

    // Verify Dependency Keys (Thread has comments)
    expect(finalHash.compositionHash).toHaveProperty('comment');

    expect(finalHash.compositionHash).toHaveProperty('_xor');
    expect(typeof finalHash.compositionHash._xor).toBe('string');

    expect(finalHash.totalHash).toBeDefined();

    // CRITICAL ASSERTION: totalHash MUST change
    expect(finalHash.totalHash).not.toBe(initialHash.totalHash);
  }, 60000);
});
