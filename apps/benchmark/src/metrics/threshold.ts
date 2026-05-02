import type { LatencySummary } from "./latency.js";

export interface RampStep {
  targetRatePerSec: number;
  achievedRatePerSec: number;
  acceptLatency: LatencySummary;
  failed: number;
  /** True if this step met the SLO (rolling p95 ≤ thresholdMs). */
  withinThreshold: boolean;
}

export interface RampResult {
  steps: RampStep[];
  /**
   * The first target rate at which p95 strictly exceeded `thresholdMs`,
   * or null if every step stayed within threshold.
   */
  rateAtThreshold: number | null;
  /** The last rate that satisfied the threshold. null if none did. */
  lastSustainedRate: number | null;
  thresholdMs: number;
}

export function summarizeRamp(steps: RampStep[], thresholdMs: number): RampResult {
  let rateAtThreshold: number | null = null;
  let lastSustainedRate: number | null = null;
  for (const s of steps) {
    if (s.withinThreshold) {
      lastSustainedRate = s.targetRatePerSec;
    } else if (rateAtThreshold === null) {
      rateAtThreshold = s.targetRatePerSec;
    }
  }
  return { steps, rateAtThreshold, lastSustainedRate, thresholdMs };
}
