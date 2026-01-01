import { createTestDb, TEST_DB_CONFIG } from './setup';
import { Surreal, Table } from 'surrealdb';

describe('Bubble Hash Logic', () => {
  let db: Surreal;

  beforeAll(async () => {
    db = await createTestDb();
  });

  afterAll(async () => {
    await db.close();
  });

  test('Hash should bubble up from Child to Parent', async () => {
    // 0. Create a User (Author)
    const user = await db.create(new Table('user')).content({
      username: 'bubble_user_' + Date.now(),
      password: 'password',
      created_at: new Date(),
    });
    const userRes = user as any;
    const userRec = Array.isArray(userRes) ? userRes[0] : userRes;
    const userId = userRec.id;

    // 1. Create a Thread (Parent)
    const threadIdVal = 'bubble_thread_' + Date.now();
    const thread = await db.create(new Table('thread')).content({
      title: 'Parent Thread ' + threadIdVal,
      content: 'Thread Content',
      author: userId,
      created_at: new Date(),
    });
    const threadRes = thread as any;
    const threadRec = Array.isArray(threadRes) ? threadRes[0] : threadRes;
    const threadId = threadRec.id;

    const getThreadHash = async () => {
      const res = (await db
        .query(`SELECT value totalHash FROM ONLY _spooky_data_hash WHERE recordId = ${threadId}`)
        .collect()) as any;
      return Array.isArray(res) ? res[0] : res;
    };

    const waitForHash = async (predicate: (hash: string) => boolean, timeout = 5000) => {
      const start = Date.now();
      while (Date.now() - start < timeout) {
        const h = await getThreadHash();
        if (h && predicate(h)) return h;
        await new Promise((r) => setTimeout(r, 200));
      }
      return await getThreadHash();
    };

    // Wait for initial hash calc
    const initialThreadHash = await waitForHash((h) => !!h);
    console.log('Initial Thread Hash:', initialThreadHash);
    expect(initialThreadHash).toBeDefined();

    // 2. Create a Comment (Child) linked to Thread
    const commentIdVal = 'bubble_comment_' + Date.now();
    const comment = await db.create(new Table('comment')).content({
      content: 'Child Content 1',
      thread: threadId,
      author: userId,
      created_at: new Date(),
    });
    const commentRes = comment as any;
    const commentRec = Array.isArray(commentRes) ? commentRes[0] : commentRes;
    const commentId = commentRec.id;

    // 3. Verify Thread Hash Changed
    const threadHashAfterChild = await waitForHash((h) => h !== initialThreadHash);
    console.log('Thread Hash After Child:', threadHashAfterChild);
    expect(threadHashAfterChild).not.toBe(initialThreadHash);

    // 4. Update Child
    await db.query(`UPDATE ${commentId} SET content = 'Child Content Updated'`);

    // 5. Verify Thread Hash Changed Again
    const threadHashAfterUpdate = await waitForHash((h) => h !== threadHashAfterChild);
    console.log('Thread Hash After Update:', threadHashAfterUpdate);
    expect(threadHashAfterUpdate).not.toBe(threadHashAfterChild);

    // 6. Delete Child
    await db.delete(commentId);

    // 7. Verify Thread Hash Reverted
    const threadHashAfterDelete = await waitForHash((h) => h === initialThreadHash);
    console.log('Thread Hash After Delete:', threadHashAfterDelete);
    expect(threadHashAfterDelete).toBe(initialThreadHash);
  }, 45000);
});
