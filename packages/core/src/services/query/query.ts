import { QueryHash, Incantation as IncantationData } from "../../types.js";
import { Table, RecordId } from "surrealdb";
import { RemoteDatabaseService } from "../database/remote.js";
import { LocalDatabaseService } from "../database/local.js";
import { Incantation } from "./incantation.js";

export class QueryManager {
  private subscriptions: Map<QueryHash, Set<(data: any) => void>> = new Map();
  // Using Map to store Incantation objects. Accessing via .values() gives "array of objects".
  private activeQueries: Map<QueryHash, Incantation> = new Map();

  constructor(private local: LocalDatabaseService, private remote: RemoteDatabaseService, private clientId?: string) {}

  async register(surrealql: string, params: Record<string, any>): Promise<QueryHash> {
    const id = await this.calculateHash({
      surrealql,
      params
    });

    const tx = await this.local.tx();
    const [incantationData] = await tx.query(`
      UPSERT _spooky_incantation:$id CONTENT {
        id: $id,
        surrealql: $surrealql,
        lastActiveAt: $lastActiveAt,
        ttl: $ttl
      };
    `, {
      id,
      surrealql,
      lastActiveAt: new Date(),
      ttl: "10m",
    }).collect<IncantationData[]>();
    await tx.commit();

    const incantationId = incantationData.id.id.toString();
    
    if (!this.activeQueries.has(incantationId)) {
      const incantation = new Incantation(incantationData, this.local, this.remote);
      this.activeQueries.set(incantationId, incantation);
      await this.initLifecycle(incantation);
    }

    return incantationId;
  }

  async queryAdHoc(surrealql: string, params: Record<string, any>, monitorId: string): Promise<QueryHash> {
      return this.register(surrealql, params);
  }

  subscribe(queryHash: QueryHash, callback: (data: any) => void): () => void {
    if (!this.subscriptions.has(queryHash)) {
      this.subscriptions.set(queryHash, new Set());
    }
    this.subscriptions.get(queryHash)!.add(callback);

    // Send current data if available
    this.refreshLocal(queryHash).then((data) => callback(data));

    return () => {
      const subs = this.subscriptions.get(queryHash);
      if (subs) {
        subs.delete(callback);
        if (subs.size === 0) {
          this.subscriptions.delete(queryHash);
          // Optional: Stop lifecycle if no subscribers.
          // For now we keep incantations alive until app closes or explicit cleanup.
        }
      }
    };
  }

  private async initLifecycle(incantation: Incantation) {
    // 1. Local Hydration
    await this.refreshLocal(incantation.id.id.toString());

    // 2. Remote Registration & Sync (moved to Incantation class)
    await incantation.init();

    // 3. Start Live Query
    await this.startLiveQuery(incantation);
  }

  private async refreshLocal(queryHash: QueryHash): Promise<any> {
    const incantation = this.activeQueries.get(queryHash);
    if (!incantation) return;

    // Cast to any to bypass potential type mismatch in alpha version
    const queryResult = await (this.local.getClient().query(incantation.surrealql) as any).collect();
    const results = queryResult[0];
    const data = results;

    // Calculate Hash
    const hash = await this.calculateHash(data);
    
    if (hash !== incantation.hash) {
      incantation.hash = hash;
      this.notifySubscribers(queryHash, data);
    }

    return data;
  }

  private async startLiveQuery(incantation: Incantation) {
    const queryUuid = await this.remote.getClient().live(
      new Table("_spooky_incantation"),
    );

    await this.remote.subscribeLive(queryUuid.toString(), async (action, result) => {
         if (action === 'UPDATE' || action === 'CREATE') {
             const resultId = (result as any)?.id?.toString();
             const targetId = incantation.id.toString();
             
             if (resultId === targetId) {
                 await this.refreshLocal(incantation.id.id.toString());
             }
         }
    });
  }

  private async calculateHash(data: any): Promise<string> {
    const result = await (this.local.getClient().query("RETURN crypto::blake3($data)", { data }) as any).collect();
    return result[0] as string;
  }

  private notifySubscribers(queryHash: QueryHash, data: any) {
    const subs = this.subscriptions.get(queryHash);
    if (subs) {
      subs.forEach((cb) => cb(data));
    }
  }
}
