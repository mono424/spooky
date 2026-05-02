/**
 * Typed wrappers for the scheduler HTTP API. See:
 *   apps/scheduler/src/query.rs:90-219    POST /view/register, /view/unregister
 *   apps/scheduler/src/ingest.rs:60-     POST /ingest
 *   apps/scheduler/src/metrics.rs:73-138 GET  /metrics, /health, /health/ready
 *   packages/ssp-protocol/src/lib.rs:1-72  wire formats
 */

export interface ViewRegisterRequest {
  id: string;
  surql: string;
  clientId: string;
  params?: Record<string, unknown>;
  ttl?: string;
  lastActiveAt?: string;
  format?: string;
}

export interface QueryAssignment {
  query_id: string;
  ssp_id: string;
  assigned_at: number;
}

export interface IngestRequest {
  table: string;
  /** "CREATE" | "UPDATE" | "DELETE" (case-insensitive on the wire) */
  op: "CREATE" | "UPDATE" | "DELETE";
  id: string;
  record: unknown;
  job_assignee?: string;
}

export interface SchedulerSspMetrics {
  id: string;
  query_count: number;
  views: number;
  cpu_usage: number | null;
  memory_usage: number | null;
  last_heartbeat_seconds_ago: number;
}

export interface SchedulerMetrics {
  scheduler: {
    total_ssps: number;
    ready_ssps: number;
    total_queries: number;
    running_jobs: number;
    uptime_seconds: number;
    pending_events: number;
    snapshot_seq: number;
    latest_seq: number;
    lag: number;
  };
  ssps: SchedulerSspMetrics[];
}

export class SchedulerClient {
  constructor(private readonly baseUrl: string) {}

  async registerView(req: ViewRegisterRequest): Promise<QueryAssignment> {
    const r = await fetch(`${this.baseUrl}/view/register`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(req),
    });
    if (!r.ok) {
      throw new Error(`register failed (${r.status}): ${await r.text()}`);
    }
    return (await r.json()) as QueryAssignment;
  }

  async unregisterView(id: string): Promise<void> {
    const r = await fetch(`${this.baseUrl}/view/unregister`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ id }),
    });
    if (!r.ok) {
      throw new Error(`unregister failed (${r.status}): ${await r.text()}`);
    }
  }

  async ingest(req: IngestRequest): Promise<void> {
    const r = await fetch(`${this.baseUrl}/ingest`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(req),
    });
    if (!r.ok) {
      throw new Error(`ingest failed (${r.status}): ${await r.text()}`);
    }
  }

  async metrics(): Promise<SchedulerMetrics> {
    const r = await fetch(`${this.baseUrl}/metrics`);
    if (!r.ok) {
      throw new Error(`/metrics failed (${r.status}): ${await r.text()}`);
    }
    return (await r.json()) as SchedulerMetrics;
  }
}
