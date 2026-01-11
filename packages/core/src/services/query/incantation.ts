import { Incantation as IncantationData, QueryTimeToLive } from '../../types.js';
import { RecordId, Duration } from 'surrealdb';

// Helper to parse duration string like "10m" to ms
function parseDuration(duration: QueryTimeToLive | Duration): number {
  if (duration instanceof Duration) {
    // Duration in surrealdb.js (check property)
    // Coerce to number to avoid BigInt mixing issues
    // Using string conversion fallback if specific props aren't reliable
    const ms = (duration as any).milliseconds || (duration as any)._milliseconds;
    if (ms) return Number(ms);

    // Fallback: try parsing string representation
    const str = duration.toString();
    if (str !== '[object Object]') return parseDuration(str as any);

    return 600000;
  }

  if (typeof duration === 'bigint') {
    return Number(duration);
  }

  if (typeof duration !== 'string') return 600000; // fallback

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
  public params?: Record<string, any>;
  public localHash: string;
  public localTree: any;
  public remoteHash: string;
  public remoteTree: any;
  public ttl: QueryTimeToLive | Duration;
  public lastActiveAt: Date | number | string;
  private ttlTimer: NodeJS.Timeout | null = null;
  private ttlDurationMs: number;
  private results: T[] | null = null;
  private meta: IncantationData['meta'];

  get records() {
    return this.results;
  }

  get tableName() {
    return this.meta.tableName;
  }

  constructor(data: IncantationData) {
    this.id = data.id;
    this.surrealql = data.surrealql;
    this.params = data.params;
    this.localHash = data.localHash;
    this.localTree = data.localTree;
    this.remoteHash = data.remoteHash;
    this.remoteTree = data.remoteTree;
    this.lastActiveAt = new Date(data.lastActiveAt);
    this.ttl = data.ttl;
    this.ttlDurationMs = parseDuration(data.ttl);
    this.meta = data.meta;
  }

  public invlovesTable(tableName: string) {
    if (this.tableName === tableName) return true;
    return this.meta.involvedTables?.includes(tableName) ?? false;
  }

  public updateLocalState(records: T[], localHash: string, localTree: any) {
    this.results = records;
    this.localHash = localHash;
    this.localTree = localTree;
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
