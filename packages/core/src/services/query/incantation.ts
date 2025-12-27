import { Incantation as IncantationData, QueryTimeToLive } from '../../types.js';
import { RecordId } from 'surrealdb';

// Helper to parse duration string like "10m" to ms
function parseDuration(duration: QueryTimeToLive): number {
  const match = duration.match(/^(\d+)([smh])$/);
  if (!match) return 600000; // default 10m
  const val = parseInt(match[1], 10);
  const unit = match[2];
  switch (unit) {
    case 's':
      return val * 1000;
    case 'h':
      return val * 3600000;
    case 'm':
    default:
      return val * 60000;
  }
}

export class Incantation<T> {
  public id: RecordId<string>;
  public surrealql: string;
  public hash: string;
  public ttl: QueryTimeToLive;
  public tree: any;
  public lastActiveAt: Date;
  private ttlTimer: NodeJS.Timeout | null = null;
  private ttlDurationMs: number;
  private results: T[] | null = null;

  constructor(data: IncantationData) {
    this.id = data.id;
    this.surrealql = data.surrealql;
    this.hash = data.hash;
    this.tree = data.tree;
    this.lastActiveAt = new Date(data.lastActiveAt);
    this.ttl = data.ttl;
    this.ttlDurationMs = parseDuration(data.ttl);
  }

  public updateLocalState(records: T[], hash: string, tree: any) {
    this.results = records;
    this.hash = hash;
    this.tree = tree;
  }

  public destroy() {
    this.stopTTLHeartbeat();
  }

  public startTTLHeartbeat(onHeartbeat: () => void) {
    if (this.ttlTimer) return;

    // Schedule next check.
    // Requirement: "call 10% before the TTL exceeds" => 90% of TTL.
    const heartbeatTime = Math.floor(this.ttlDurationMs * 0.9);

    // Ensure we don't spam if TTL is very short, but for "10m" (600s), 90% is 540s.
    this.ttlTimer = setTimeout(() => {
      onHeartbeat();
      this.startTTLHeartbeat(onHeartbeat);
    }, heartbeatTime);
  }

  private stopTTLHeartbeat() {
    if (this.ttlTimer) {
      clearTimeout(this.ttlTimer);
      this.ttlTimer = null;
    }
  }
}
