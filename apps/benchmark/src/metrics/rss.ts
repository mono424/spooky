import { execa } from "execa";
import { sleep } from "../util/wait.js";

export interface RssSample {
  /** ms since the sampler started */
  t: number;
  /** Resident set size in KiB (matches `ps -o rss=`) */
  rssKib: number;
}

export class RssSampler {
  private readonly samples: RssSample[] = [];
  private stopped = false;
  private startedAt = 0;
  private task: Promise<void> | null = null;

  constructor(
    private readonly pid: number,
    private readonly intervalMs: number = 250,
  ) {}

  start(): void {
    if (this.task) return;
    this.startedAt = Date.now();
    this.task = (async () => {
      while (!this.stopped) {
        const rss = await readRss(this.pid);
        if (rss !== null) {
          this.samples.push({ t: Date.now() - this.startedAt, rssKib: rss });
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

  getSamples(): RssSample[] {
    return [...this.samples];
  }

  summary(): { samples: RssSample[]; minKib: number; maxKib: number; lastKib: number } {
    if (this.samples.length === 0) {
      return { samples: [], minKib: 0, maxKib: 0, lastKib: 0 };
    }
    let min = Infinity;
    let max = -Infinity;
    for (const s of this.samples) {
      if (s.rssKib < min) min = s.rssKib;
      if (s.rssKib > max) max = s.rssKib;
    }
    return {
      samples: this.samples,
      minKib: min,
      maxKib: max,
      lastKib: this.samples[this.samples.length - 1]!.rssKib,
    };
  }
}

async function readRss(pid: number): Promise<number | null> {
  // `ps -o rss= -p <pid>` works on Linux and macOS. Output is RSS in KiB.
  try {
    const { stdout, exitCode } = await execa("ps", ["-o", "rss=", "-p", String(pid)], {
      reject: false,
    });
    if (exitCode !== 0) return null;
    const trimmed = stdout.trim();
    if (!trimmed) return null;
    const kib = Number.parseInt(trimmed.split(/\s+/)[0]!, 10);
    return Number.isFinite(kib) ? kib : null;
  } catch {
    return null;
  }
}
