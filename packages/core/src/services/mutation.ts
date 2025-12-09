import { DatabaseService } from "./database.js";
import { Table, RecordId } from "surrealdb";

export class MutationManager {
  constructor(private db: DatabaseService) {}

  async create<T extends Record<string, unknown>>(
    table: string,
    data: T
  ): Promise<T> {
    // Perform creation on local DB
    const result = await this.db.queryLocal<T[]>(
      `CREATE type::table($table) CONTENT $data`, 
      { table, data }
    );
    const created = (Array.isArray(result) ? result[0] : result) as T;
    
    // Sync to remote (optimistic or queue-based)
    this.db.queryRemote(`CREATE type::table($table) CONTENT $data`, { table, data }).catch(console.error);

    return created;
  }

  async update<T extends Record<string, unknown>>(
    table: string,
    id: string,
    data: Partial<T>
  ): Promise<T> {
    // Use query for merge to avoid type issues if merge method is missing/changed
    const rid = new RecordId(table, id);
    const result = await this.db.queryLocal<T[]>(
      `UPDATE $id MERGE $data`, 
      { id: rid, data }
    );
    const updated = (Array.isArray(result) ? result[0] : result) as T;
    
    this.db.queryRemote(`UPDATE $id MERGE $data`, { id: rid, data }).catch(console.error);

    return updated;
  }

  async delete(table: string, id: string): Promise<void> {
    const rid = new RecordId(table, id);
    await this.db.getLocal().delete(rid);
    
    this.db.getRemote().delete(rid).catch(console.error);
  }
}
