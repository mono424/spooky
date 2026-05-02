import type { SchedulerClient, SchedulerMetrics } from "../drivers/scheduler-http.js";
import { sleep } from "../util/wait.js";

export interface SchedulerSnapshot {
  t: number;
  metrics: SchedulerMetrics;
}

export class SchedulerStatsPoller {
  private readonly snapshots: SchedulerSnapshot[] = [];
  private stopped = false;
  private startedAt = 0;
  private task: Promise<void> | null = null;

  constructor(
    private readonly client: SchedulerClient,
    private readonly intervalMs: number = 1000,
  ) {}

  start(): void {
    if (this.task) return;
    this.startedAt = Date.now();
    this.task = (async () => {
      while (!this.stopped) {
        try {
          const m = await this.client.metrics();
          this.snapshots.push({ t: Date.now() - this.startedAt, metrics: m });
        } catch {
          /* transient, keep polling */
        }
        await sleep(this.intervalMs);
      }
    })();
  }

  async stop(): Promise<void> {
    this.stopped = true;
    if (this.task) await this.task.catch(() => {});
    this.task = null;
  }

  /** Force a synchronous extra snapshot; useful at end-of-step. */
  async captureNow(): Promise<void> {
    const m = await this.client.metrics();
    this.snapshots.push({
      t: this.startedAt > 0 ? Date.now() - this.startedAt : 0,
      metrics: m,
    });
  }

  getSnapshots(): SchedulerSnapshot[] {
    return [...this.snapshots];
  }

  last(): SchedulerSnapshot | null {
    return this.snapshots.at(-1) ?? null;
  }
}
