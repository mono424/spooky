import { DatabaseService } from "./database.js";
import { QueryHash, Incantation } from "../types.js";
import { Table } from "surrealdb";

export class QueryManager {
  private subscriptions: Map<QueryHash, Set<(data: any) => void>> = new Map();
  private activeQueries: Map<QueryHash, Incantation> = new Map();

  constructor(private db: DatabaseService) {}

  async register(surrealql: string): Promise<QueryHash> {
    // 1. Calculate Query Hash (ID)
    // We use a simple hash of the query string for now. 
    // In a real implementation, this should be more robust.
    const queryHash = this.hashString(surrealql);

    if (this.activeQueries.has(queryHash)) {
      return queryHash;
    }

    // 2. Local Initialization
    const incantation: Incantation = {
      id: queryHash,
      surrealql,
      hash: 0,
      lastActiveAt: Date.now(),
    };
    this.activeQueries.set(queryHash, incantation);

    // 3. Start Lifecycle
    await this.initLifecycle(incantation);

    return queryHash;
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
