import { createTestDb, TEST_DB_CONFIG } from './setup';
import { Surreal } from 'surrealdb';

describe('Blake3 XOR Logic', () => {
  let db: Surreal;

  beforeAll(async () => {
    db = await createTestDb();
  });

  afterAll(async () => {
    await db.close();
  });

  test('should XOR two hashes correctly', async () => {
    // Real WASM implementation: mod::xor::blake3_xor
    
    const hashA = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
    const zeroHash = "0000000000000000000000000000000000000000000000000000000000000000";
    
    // Test 1: XOR same hash -> 0
    // Because A XOR A = 0
    const resultSame = await db.query(`RETURN mod::xor::blake3_xor('${hashA}', '${hashA}');`).collect() as any;
    const valSame = Array.isArray(resultSame) ? resultSame[0] : resultSame;
    expect(valSame).toBe(zeroHash); 

    // Test 2: Neutral element (A XOR 0 = A)
    const resultNeutral = await db.query(`RETURN mod::xor::blake3_xor('${hashA}', '${zeroHash}');`).collect() as any;
    const valNeutral = Array.isArray(resultNeutral) ? resultNeutral[0] : resultNeutral;
    expect(valNeutral).toBe(hashA);
  
    // Test 3: Commutativity (A XOR B = B XOR A)
    const hashB = "2b1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
    const resAB = await db.query(`RETURN mod::xor::blake3_xor('${hashA}', '${hashB}');`).collect() as any;
    const resBA = await db.query(`RETURN mod::xor::blake3_xor('${hashB}', '${hashA}');`).collect() as any;
    const valAB = Array.isArray(resAB) ? resAB[0] : resAB;
    const valBA = Array.isArray(resBA) ? resBA[0] : resBA;
    
    expect(valAB).toBe(valBA);
    expect(valAB).not.toBe(hashA);
    expect(valAB).not.toBe(hashB);
  });
});
