export const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

export async function waitFor(
  predicate: () => Promise<boolean>,
  opts: { timeoutMs: number; intervalMs?: number; label?: string },
): Promise<void> {
  const interval = opts.intervalMs ?? 200;
  const deadline = Date.now() + opts.timeoutMs;
  let lastErr: unknown;
  while (Date.now() < deadline) {
    try {
      if (await predicate()) return;
    } catch (e) {
      lastErr = e;
    }
    await sleep(interval);
  }
  throw new Error(
    `Timed out after ${opts.timeoutMs}ms waiting for ${opts.label ?? "condition"}` +
      (lastErr ? `: ${(lastErr as Error).message ?? lastErr}` : ""),
  );
}

export async function waitForHttp(
  url: string,
  opts: { timeoutMs: number; intervalMs?: number; expectStatus?: number; label?: string } = {
    timeoutMs: 30_000,
  },
) {
  const expect = opts.expectStatus ?? 200;
  await waitFor(
    async () => {
      const ctrl = new AbortController();
      const t = setTimeout(() => ctrl.abort(), 1000);
      try {
        const r = await fetch(url, { signal: ctrl.signal });
        return r.status === expect;
      } finally {
        clearTimeout(t);
      }
    },
    { timeoutMs: opts.timeoutMs, intervalMs: opts.intervalMs ?? 250, label: opts.label ?? `HTTP ${expect} on ${url}` },
  );
}
