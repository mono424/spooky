
import { Surreal } from 'surrealdb';
import { createTestDb, TEST_DB_CONFIG } from './setup';

let db: Surreal;
const testNs = 'main';
const testDb = 'main';

describe('Authentication Flows', () => {
    beforeAll(async () => {
        db = await createTestDb();
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

    test('Should have Access "account" defined', async () => {
        const info = await db.query('INFO FOR DB');
        console.log("INFO FOR DB:", JSON.stringify(info));
        // Check if output contains "account"
    });

    test('Should support crypto::argon2', async () => {
        try {
           const hash = await db.query('RETURN crypto::argon2::generate("test")');
           console.log("Crypto Test:", hash);
        } catch (e) {
            console.error("Crypto Failed:", e);
            throw e;
        }
    });

    test('Should Signup a new user', async () => {
        // const uniqueSuffix = Date.now().toString();
        // const username = `testuser_${uniqueSuffix}`;
        // const password = "securepassword";
        
        try {
            await db.invalidate();
            // Ensure we are using the correct DB context as Guest
            await db.use({ namespace: TEST_DB_CONFIG.namespace, database: TEST_DB_CONFIG.database });
            
            const username = `test_user_${Date.now()}`;
            const password = 'password123';
            
            // Manual creation (Signup)
            // Guest has permissions to CREATE user and use crypto::argon2::generate
            const createQuery = `
                CREATE ONLY user CONTENT {
                    username: $username,
                    password: crypto::argon2::generate($password),
                    created_at: time::now()
                }
            `;
            await db.query(createQuery, { username, password });

            // Signin to get the token
            const token = await db.signin({
                namespace: TEST_DB_CONFIG.namespace,
                database: TEST_DB_CONFIG.database,
                access: 'account',
                variables: {
                    username,
                    password,
                },
            });
            // console.log("Signup Token:", token);
            expect(token).toBeDefined();
        } catch (e: any) {
            console.error("Signup Error:", e);
            if (e.cause) console.error("Signup Error Cause:", e.cause);
            throw e;
        }
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
