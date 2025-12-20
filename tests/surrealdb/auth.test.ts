
import { Surreal } from 'surrealdb';
import { createTestDb } from './setup';

let db: Surreal;
const testNs = 'main';
const testDb = 'main';

describe('Authentication Flows', () => {
    beforeAll(async () => {
        db = await createTestDb();
        // Ensure we use the main database where schema is applied
        await db.use({ namespace: testNs, database: testDb });

        const accessQuery = `
            DEFINE ACCESS account ON DATABASE TYPE RECORD
            SIGNUP {
                // Standard signup should now work with events enabled because hash table is writable by guest
                LET $u = (CREATE user CONTENT {
                    username: $username,
                    password: crypto::argon2::generate($password),
                    created_at: time::now()
                });
                RETURN $u;
            }
            SIGNIN ( SELECT * FROM user WHERE username = $username AND crypto::argon2::compare(password, $password) )
            DURATION FOR TOKEN 365d, FOR SESSION 365d;
        `;
        try {
            // Remove first to allow redefinition
            try { await db.query('REMOVE ACCESS account ON DATABASE'); } catch(e) {}
            await db.query(accessQuery);
        } catch (e: any) {
             console.warn("Failed to apply ACCESS definition (might already exist):", e.message);
        }
    });

    afterAll(async () => {
        await db.close();
    });
    
    // Shared user for signin test
    const sharedUser = "signin_manual_user_" + Date.now();
    const sharedPass = "password";

    test('Should manually create user (Root Debug)', async () => {
        const username = sharedUser;
        const password = sharedPass;
        // Run specific query to emulate signup logic
        const query = `
            CREATE user CONTENT {
                username: $username,
                password: crypto::argon2::generate($password),
                created_at: time::now()
            }
        `;
        const result = await db.query(query, { username, password }).collect();
        const created = result[0] as any;
        expect(created).toBeDefined();
        if (created && created[0]) {
             // console.log("Created user id:", created[0].id);
        }
    });

    test('Should Signup a new user', async () => {
        // const uniqueSuffix = Date.now().toString();
        // const username = `testuser_${uniqueSuffix}`;
        // const password = "securepassword";
        
        // let token;
        try {
            const token = await db.signup({
                namespace: testNs,
                database: testDb,
                access: 'account',
                variables: {
                    username: `test_user_${Date.now()}`,
                    password: 'password123',
                },
            });
            // console.log("Signup Token:", token);
            expect(token).toBeDefined();
        } catch (e: any) {
            console.error("Signup Error:", e);
            if (e.cause) console.error("Signup Error Cause:", e.cause);
            throw e;
        }

        // expect(token).toBeDefined();
        // console.log("Signup Token:", token);
    });

    test('Should Signin an existing user', async () => {
        // Use the manually created user from first test
        const username = sharedUser;
        const password = sharedPass;

        // Invalidate and Signin
        await db.invalidate();
        
        try {
            const token = await db.signin({
                access: 'account',
                variables: {
                    username: username,
                    password: password,
                }
            });
            // console.log("Signin Token:", token);
            expect(token).toBeDefined();
        } catch (e: any) {
            console.error("Signin Failed:", e);
            throw e;
        }
    });
});
