/* prettier-ignore */

// --- Authentication Snippets ---

export const authSchemaCode = `-- Define access scope for accounts
DEFINE ACCESS account ON DATABASE TYPE RECORD
  SIGNUP ( 
    CREATE user 
    SET username = $username,
      password = crypto::argon2::generate($password)
  )
  SIGNIN (
    SELECT * FROM user 
    WHERE username = $username 
      AND crypto::argon2::compare(password, $password)
  )
  DURATION FOR TOKEN 15m, FOR SESSION 30d;

-- Permissions use the $auth variable to check the current user
DEFINE TABLE user SCHEMAFULL
  PERMISSIONS
    FOR update, delete WHERE id = $auth.id
    FOR select, create WHERE true;`;

export const authSignInCode = `import { db } from './db';

// The second argument is typesafe based on your SIGNIN definition
await db.auth.signIn('account', {
  username: 'spooky_user',
  password: 'secure_password'
});`;

export const authSignUpCode = `import { db } from './db';

await db.auth.signUp('account', {
  username: 'new_user',
  password: 'secure_password'
});`;

export const authStateCode = `// Subscribe to auth changes
db.auth.subscribe((userId) => {
  if (userId) {
    console.log('User logged in:', userId);
  } else {
    console.log('User logged out');
  }
});

// Sign out
await db.auth.signOut();`;

// --- Query Data Snippets ---

export const querySimpleCode = `// Simple query for a list of threads
const threads = await db.query('thread')
  .orderBy('created_at', 'desc')
  .limit(10)
  .build();`;

export const queryUnifiedCode = `// 1:1 Relationship (User -> Profile)
const userWithProfile = await db.query('user')
  .related('profile')
  .build();

// 1:N Relationship (Thread -> Author)
const threads = await db.query('thread')
  .related('author')
  .build();

// N:M Relationship (User -> Liked Posts)
// Even for graph edges, you just specify the field name corresponding to the relation
const usersWithLikes = await db.query('user')
  .related('liked')
  .build();`;

export const queryTypeSafetyCode = `// 1. Basic Query
const basicThreads = await db.query('thread').build();
// basicThreads[0].author IS A RecordId

// 2. Relational Query
const fullThreads = await db.query('thread').related('author').build();
// fullThreads[0].author IS A User object (with all its fields)`;

export const queryHooksCode = `import { useQuery } from '@spooky/client-solid';
import { db } from './db';

function ThreadList() {
  // This query will automatically update "threads" whenever the database changes
  const queryResult = useQuery(db, () => {
    return db.query('thread')
      .related('author')
      .orderBy('title', 'desc')
      .limit(10)
      .build();
  });

  const threads = () => queryResult.data() || [];

  return (
    <ul>
      <For each={threads()}>
        {(thread) => (
          <li>{thread.title} by {thread.author?.username}</li>
        )}
      </For>
    </ul>
  );
}`;

// --- Mutate Data Snippets ---

export const mutateCreateCode = `import { Uuid } from '@spooky/client-solid';
import { RecordId } from 'surrealdb';

// Generate a unique ID
const genId = Uuid.v4().toString().replace(/-/g, '');
const threadId = \`thread:\${genId}\`;

// Create the record
await db.create(threadId, {
  title: 'My New Thread',
  content: 'This is the content of the thread.',
  author: new RecordId('user', 'current_user_id'),
  active: true,
  created_at: new Date(),
});`;

export const mutateUpdateCode = `// Partial update (merge)
await db.merge('thread:123', {
  title: 'Updated Title'
});

// Full update (replace)
await db.update('thread:123', {
  id: 'thread:123',
  title: 'Updated Title',
  content: 'New content...',
  // ... must provide all required fields
});`;

export const mutateDeleteCode = `await db.delete('thread:123');`;
