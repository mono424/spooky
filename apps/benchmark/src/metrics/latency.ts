export interface LatencySummary {
  count: number;
  min: number;
  max: number;
  mean: number;
  p50: number;
  p95: number;
  p99: number;
  /** All raw samples in ms (sorted ascending). */
  samples: number[];
}

export class LatencyRecorder {
  private readonly samples: number[] = [];

  record(ms: number): void {
    this.samples.push(ms);
  }

  count(): number {
    return this.samples.length;
  }

  summarize(): LatencySummary {
    const sorted = [...this.samples].sort((a, b) => a - b);
    if (sorted.length === 0) {
      return { count: 0, min: 0, max: 0, mean: 0, p50: 0, p95: 0, p99: 0, samples: [] };
    }
    const sum = sorted.reduce((a, b) => a + b, 0);
    return {
      count: sorted.length,
      min: sorted[0]!,
      max: sorted[sorted.length - 1]!,
      mean: sum / sorted.length,
      p50: percentile(sorted, 0.5),
      p95: percentile(sorted, 0.95),
      p99: percentile(sorted, 0.99),
      samples: sorted,
    };
  }
}

function percentile(sorted: number[], p: number): number {
  if (sorted.length === 0) return 0;
  // Nearest-rank method, clamped.
  const rank = Math.min(sorted.length - 1, Math.max(0, Math.ceil(p * sorted.length) - 1));
  return sorted[rank]!;
}
