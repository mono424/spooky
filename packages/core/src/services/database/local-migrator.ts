import type { Surreal, SurrealTransaction } from "surrealdb";
import { LocalDatabaseService } from "./local.js";

export interface SchemaRecord {
  hash: string;
  created_at: string;
}

export const sha1 = async (str: string): Promise<string> => {
  const enc = new TextEncoder();
  const hash = await crypto.subtle.digest("SHA-1", enc.encode(str));
  return Array.from(new Uint8Array(hash))
    .map((v) => v.toString(16).padStart(2, "0"))
    .join("");
};

export class LocalMigrator {

  constructor(private localDb: LocalDatabaseService) {}

  async provision(schemaSurql: string): Promise<void> {
    const hash = await sha1(schemaSurql);

    const { database } = this.localDb.getConfig();
    const tx = await this.localDb.tx();

    if (await this.isSchemaUpToDate(tx, hash)) {
      console.info("[Provisioning] Schema is up to date, skipping migration");
      return;
    }

    await this.recreateDatabase(tx, database);

    const statements = schemaSurql
      .split(";")
      .map((s) => s.trim())
      .filter((s) => s.length > 0);

    for (let i = 0; i < statements.length; i++) {
       const statement = statements[i];
       try {
         await tx.query(statement);
         console.info(`[Provisioning] (${i + 1}/${statements.length}) Executed: ${statement.substring(0, 50)}...`);
       } catch (e) {
         console.error(`[Provisioning] (${i + 1}/${statements.length}) Error executing statement: ${statement}`);
         throw e;
       }
    }
    
    await this.createHashRecord(tx, hash);
    await tx.commit();
  }

  private async isSchemaUpToDate(tx: SurrealTransaction, hash: string): Promise<boolean> {
    try {
      const response = await tx.query(
        `SELECT hash, created_at FROM _spooky_schema ORDER BY created_at DESC LIMIT 1;`
      ).collect<SchemaRecord[]>();

      const [lastSchemaRecord] = response;
      return lastSchemaRecord?.hash === hash;
    } catch (error) {
      return false;
    }
  }

  private async recreateDatabase(tx: SurrealTransaction, database: string) {
    await tx.query((`
      USE DB _spooky_temp;
      REMOVE DATABASE ${database};
      DEFINE DATABASE ${database};
      USE DB ${database};
    `));
  }

  private async createHashRecord(tx: SurrealTransaction, hash: string) {
    await tx.query(
      `UPSERT _spooky_schema SET hash = $hash, created_at = time::now() WHERE hash = $hash;`,
      { hash }
    );
  }
}
