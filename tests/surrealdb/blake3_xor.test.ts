import { createTestDb, TEST_DB_CONFIG } from './setup';
import { Surreal } from 'surrealdb';

describe('Blake3 XOR Logic', () => {
  let db: Surreal;

  beforeAll(async () => {
    db = await createTestDb();
    // We use 'main' database because it has the modules loaded (mod::xor)
    await db.use({ namespace: 'main', database: 'main' });
  });

  afterAll(async () => {
    await db.close();
  });

  test('should XOR two hashes correctly', async () => {
    // Define a simple XOR test
    // XOR of identical things is 0 (or equivalent usage in the module)
    // XOR with 0 is identity
    
    // We need to know the valid input format for blake3_xor. presumably strings (hashes)?
    
    // Test 1: XOR same hash -> should be zero-like or empty
    // Queries can be run directly
    const hashA = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"; // Some random blake3 hash
    const resultSame = await db.query(`RETURN mod::xor::blake3_xor('${hashA}', '${hashA}');`).collect() as any;
    // Expected result: 32 bytes of zeros encoded as hex? 
    // 0000000000000000000000000000000000000000000000000000000000000000
    // Query returns array of results?
    // [ "000..." ] or [ { result: "000..." } ]? usually [ "result_value" ] for RETURN
    // collect() converts the AsyncIterable of results to an array.
    // So resultSame is [ "000..." ].
    // Wait, query can return multiple results if multiple statements.
    // resultSame[0] is the result of the first statement.
    const val = Array.isArray(resultSame) ? resultSame[0] : resultSame;
    expect(val).toBe("0000000000000000000000000000000000000000000000000000000000000000");

    // Test 2: Neutral element
    const zeroHash = "0000000000000000000000000000000000000000000000000000000000000000";
    const resultNeutral = await db.query(`RETURN mod::xor::blake3_xor('${hashA}', '${zeroHash}');`).collect() as any;
    const valNeutral = Array.isArray(resultNeutral) ? resultNeutral[0] : resultNeutral;
    expect(valNeutral).toBe(hashA);
  
    // Test 3: Commutativity
    const hashB = "2b1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
    const resAB = await db.query(`RETURN mod::xor::blake3_xor('${hashA}', '${hashB}');`).collect() as any;
    const resBA = await db.query(`RETURN mod::xor::blake3_xor('${hashB}', '${hashA}');`).collect() as any;
    const valAB = Array.isArray(resAB) ? resAB[0] : resAB;
    const valBA = Array.isArray(resBA) ? resBA[0] : resBA;
    expect(valAB).toBe(valBA);
  });
});
