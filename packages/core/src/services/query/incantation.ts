import { Incantation as IncantationData } from "../../types.js";
import { RecordId } from "surrealdb";
import { RemoteDatabaseService } from "../database/remote.js";
import { LocalDatabaseService } from "../database/local.js";

// Helper to parse duration string like "10m" to ms
function parseDuration(duration: string): number {
    const match = duration.match(/^(\d+)([smh])$/);
    if (!match) return 600000; // default 10m
    const val = parseInt(match[1], 10);
    const unit = match[2];
    switch (unit) {
        case 's': return val * 1000;
        case 'h': return val * 3600000;
        case 'm': 
        default: return val * 60000;
    }
}

export class Incantation {
    public id: RecordId<string>;
    public surrealql: string;
    public hash: number;
    public lastActiveAt: Date;
    public ttl: string; // "10m"
    private ttlTimer: NodeJS.Timeout | null = null;
    private ttlDurationMs: number;

    constructor(
        data: IncantationData,
        private local: LocalDatabaseService,
        private remote: RemoteDatabaseService
    ) {
        this.id = data.id;
        this.surrealql = data.surrealql;
        this.hash = data.hash;
        // Ensure Date object
        this.lastActiveAt = new Date(data.lastActiveAt);
        this.ttl = "10m"; // Default or from data if available
        this.ttlDurationMs = parseDuration(this.ttl);
    }

    public async init() {
         // 1. Local Hydration (ensure we have data locally? QueryManager does this currently)
         // Actually QueryManager calls initLifecycle which calls this.
         // Let's keep the logic distributed for now or move it here.
         // The requirement "Incantation class that maintains the remote registered state"
         // suggests moving registerRemote here.
         await this.registerRemote();
         this.startTTLHeartbeat();
    }

    public destroy() {
        this.stopTTLHeartbeat();
    }

    private async registerRemote() {
        // Check if incantation exists remotely
        const queryResult = await (this.remote.getClient().query(
            `SELECT * FROM _spooky_incantation WHERE id = $id`,
            { id: this.id }
        ) as any).collect();
        const remoteIncantations = queryResult[0] as IncantationData[];

        if (!remoteIncantations || remoteIncantations.length === 0) {
            await (this.remote.getClient().query(`
                UPSERT _spooky_incantation:$id CONTENT {
                    id: $id,
                    surrealql: $surrealql,
                    lastActiveAt: $lastActiveAt,
                    ttl: $ttl
                };
            `, {
                id: this.id,
                surrealql: this.surrealql,
                lastActiveAt: this.lastActiveAt,
                ttl: this.ttl 
            }) as any).collect();
        }
    }

    private startTTLHeartbeat() {
        if (this.ttlTimer) return;

        // Schedule next check.
        // Requirement: "call 10% before the TTL exceeds" => 90% of TTL.
        const heartbeatTime = Math.floor(this.ttlDurationMs * 0.9);
        
        // Ensure we don't spam if TTL is very short, but for "10m" (600s), 90% is 540s.
        this.ttlTimer = setTimeout(() => {
            this.enlargeTTL();
        }, heartbeatTime);
    }

    private stopTTLHeartbeat() {
        if (this.ttlTimer) {
            clearTimeout(this.ttlTimer);
            this.ttlTimer = null;
        }
    }

    private async enlargeTTL() {
        // Reset timer immediately to avoid double calls? 
        // Or wait for success? Better to just clear and re-schedule.
        this.ttlTimer = null;

        try {
            this.lastActiveAt = new Date();
            // Update remote
            await (this.remote.getClient().query(`
                UPDATE _spooky_incantation:$id SET lastActiveAt = $now
            `, {
                id: this.id,
                now: this.lastActiveAt
            }) as any).collect();
            
            // Re-schedule
            this.startTTLHeartbeat();
        } catch (e) {
            console.error("Failed to enlarge TTL for incantation", this.id, e);
            // Retry sooner? Or just fail. 
            // Let's retry in 10s if failed? 
            // For simplicity, just try again in 1 min or standard flow.
            // If we don't reschedule, it dies.
            // Let's reschedule for short retry.
             this.ttlTimer = setTimeout(() => this.enlargeTTL(), 30000);
        }
    }
}
