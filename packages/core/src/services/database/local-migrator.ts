import type { Surreal } from 'surrealdb';
import { LocalDatabaseService } from './local.js';

export interface SchemaRecord {
  hash: string;
  created_at: string;
}

export const sha1 = async (str: string): Promise<string> => {
  const enc = new TextEncoder();
  const hash = await crypto.subtle.digest('SHA-1', enc.encode(str));
  return Array.from(new Uint8Array(hash))
    .map((v) => v.toString(16).padStart(2, '0'))
    .join('');
};

export class LocalMigrator {
  constructor(private localDb: LocalDatabaseService) {}

  async provision(schemaSurql: string): Promise<void> {
    const hash = await sha1(schemaSurql);

    const { database } = this.localDb.getConfig();

    if (await this.isSchemaUpToDate(hash)) {
      console.info('[Provisioning] Schema is up to date, skipping migration');
      return;
    }

    await this.recreateDatabase(database);

    const statements = schemaSurql
      .split(';')
      .map((s) => s.trim())
      .filter((s) => s.length > 0);

    for (let i = 0; i < statements.length; i++) {
      const statement = statements[i];
      try {
        await this.localDb.query(statement);
        console.info(
          `[Provisioning] (${i + 1}/${statements.length}) Executed: ${statement.substring(0, 50)}...`
        );
      } catch (e) {
        console.error(
          `[Provisioning] (${i + 1}/${statements.length}) Error executing statement: ${statement}`
        );
        throw e;
      }
    }

    await this.createHashRecord(hash);
  }

  private async isSchemaUpToDate(hash: string): Promise<boolean> {
    try {
      const response = await this.localDb.query<SchemaRecord[]>(
        `SELECT hash, created_at FROM _spooky_schema ORDER BY created_at DESC LIMIT 1;`
      );

      const [lastSchemaRecord] = response;
      return lastSchemaRecord?.hash === hash;
    } catch (error) {
      return false;
    }
  }

  private async recreateDatabase(database: string) {
    // Ensure temp db exists so we can switch to it
    await this.localDb.query(`DEFINE DATABASE _spooky_temp;`);

    try {
      await this.localDb.query(`
        USE DB _spooky_temp;
        REMOVE DATABASE ${database};
      `);
    } catch (e) {
      // Ignore error if database doesn't exist
    }

    await this.localDb.query(`
      DEFINE DATABASE ${database};
      USE DB ${database};
    `);
  }

  private async createHashRecord(hash: string) {
    await this.localDb.query(
      `UPSERT _spooky_schema SET hash = $hash, created_at = time::now() WHERE hash = $hash;`,
      { hash }
    );
  }
}
