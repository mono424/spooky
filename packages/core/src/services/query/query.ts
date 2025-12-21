import { QueryHash, Incantation } from "../../types.js";
import { Table } from "surrealdb";
import { RemoteDatabaseService } from "../database/remote.js";
import { LocalDatabaseService } from "../database/local.js";

export class QueryManager {
  private subscriptions: Map<QueryHash, Set<(data: any) => void>> = new Map();
  private activeQueries: Map<QueryHash, Incantation> = new Map();

  constructor(private local: LocalDatabaseService, private remote: RemoteDatabaseService) {}

  async register(surrealql: string, params: Record<string, any>): Promise<QueryHash> {
    const tx = await this.local.tx();
    const [incantation] = await tx.query(`
      LET $id = crypto::blake3({
        surrealql: $surrealql,
        params: $params
      });
      UPSERT _spooky_incantation:$id CONTENT {
        Id: $id,
        SurrealQL: $surrealql,
        LastActiveAt: $lastActiveAt,
        TTL: $ttl
      };
    `, {
      surrealql,
      params,
      lastActiveAt: new Date(),
      ttl: "10m",
    }).collect<Incantation[]>();
    await tx.commit();

    const incantationId = incantation.id.id.toString()
    if (!this.activeQueries.has(incantationId)) {
      this.activeQueries.set(incantationId, incantation);
      this.initLifecycle(incantation);
    }

    return incantationId;
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
          // Optional: Stop lifecycle if no subscribers
        }
      }
    };
  }

  private async initLifecycle(incantation: Incantation) {
    // 1. Local Hydration
    await this.refreshLocal(incantation.id);

    // 2. Remote Registration & Sync
    await this.registerRemote(incantation);

    // 3. Start Live Query
    await this.startLiveQuery(incantation);
  }

  private async refreshLocal(queryHash: QueryHash): Promise<any> {
    const incantation = this.activeQueries.get(queryHash);
    if (!incantation) return;

    const results = await this.db.queryLocal<any[]>(incantation.surrealql);
    const data = results;

    // Calculate Hash
    const hash = await this.calculateHash(data);
    
    if (hash !== incantation.hash) {
      incantation.hash = hash;
      this.notifySubscribers(queryHash, data);
    }

    return data;
  }

  private async registerRemote(incantation: Incantation) {
    // Check if incantation exists remotely
    // This is a simplified version of the README's "Remote registration"
    const remoteIncantation = await this.db.queryRemote<Incantation[]>(
      `SELECT * FROM spooky_incantation WHERE id = $id`,
      { id: incantation.id }
    );

    if (!remoteIncantation || remoteIncantation.length === 0) {
      await this.db.queryRemote(`CREATE spooky_incantation CONTENT $data`, {
        data: {
          id: incantation.id,
          surrealql: incantation.surrealql,
          hash: 0, // Initial hash
        }
      });
    }
  }

  private async startLiveQuery(incantation: Incantation) {
    // Listen to changes on the spooky_incantation table for this specific ID
    // or listen to the actual query if supported.
    // The README says: "First, we set up a LIVE QUERY that listens to that remote Incantation"
    
    const subscription = await this.db.getRemote().live(
      new Table("spooky_incantation"),
    );

    // subscription is likely async iterable
    (async () => {
        try {
            // @ts-ignore
            for await (const msg of subscription) {
                // Handle update
                // For now just log or something, or trigger sync.
                // console.log("Live update", msg);
            }
        } catch (e) {
            console.error(e);
        }
    })();
  }

  private async calculateHash(data: any): Promise<number> {
    // Use DB-native hashing if possible, or a simple JS hash for now
    const str = JSON.stringify(data);
    let hash = 0;
    for (let i = 0; i < str.length; i++) {
      const char = str.charCodeAt(i);
      hash = (hash << 5) - hash + char;
      hash = hash & hash; // Convert to 32bit integer
    }
    return hash;
  }

  private hashString(str: string): number {
    let hash = 0;
    for (let i = 0; i < str.length; i++) {
      const char = str.charCodeAt(i);
      hash = (hash << 5) - hash + char;
      hash = hash & hash;
    }
    return hash;
  }

  private notifySubscribers(queryHash: QueryHash, data: any) {
    const subs = this.subscriptions.get(queryHash);
    if (subs) {
      subs.forEach((cb) => cb(data));
    }
  }
}
