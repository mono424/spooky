import { describe, it, expect } from "vitest";
import { Surreal } from "surrealdb";
import { createNodeEngines } from "@surrealdb/node";

describe("Hashing Utilities", () => {
    it("should calculate BLAKE3 hash using Node engine", async () => {
        const db = new Surreal({
            engines: createNodeEngines(),
        });
        await db.connect("mem://");

        // Test crypto::blake3
        // In SurrealDB 2.0, RETURN statement returns the value directly
        const result = await db.query(`RETURN crypto::blake3("test")`).collect();

        // "test" blake3 hash: 4878ca0425c739fa427f7bf20f4959c67ac6e37d7eee1661f3303417ff0206e7
        console.log("BLAKE3 Result:", result);

        expect(result[0]).toBe("4878ca0425c739fa427f7eda20fe845f6b2e46ba5fe2a14df5b1e32f50603215");

        await db.close();
    }, 30000);
});
