/**
 * Direct client for the SSP's authenticated HTTP routes (apps/ssp/src/lib.rs:296-311).
 * All authenticated routes require `Authorization: Bearer <SPKY_AUTH_SECRET>`.
 */

export interface DebugViewResponse {
  view_id: string;
  cache_size: number;
  /** Stable content hash that changes whenever the materialized view changes. */
  last_hash: string;
  format: string;
  cache: Array<{ key: string; weight: number }>;
  content_generation: number;
}

export class SspClient {
  constructor(
    private readonly baseUrl: string,
    private readonly authSecret: string,
  ) {}

  private headers(): Record<string, string> {
    return { authorization: `Bearer ${this.authSecret}` };
  }

  /** Read materialized view state. Returns null if the view isn't registered yet. */
  async getDebugView(viewId: string): Promise<DebugViewResponse | null> {
    const r = await fetch(`${this.baseUrl}/debug/view/${encodeURIComponent(viewId)}`, {
      headers: this.headers(),
    });
    if (!r.ok) {
      throw new Error(`/debug/view/${viewId} failed (${r.status}): ${await r.text()}`);
    }
    const body = (await r.json()) as DebugViewResponse | { error: string };
    if ("error" in body) return null;
    return body;
  }
}
