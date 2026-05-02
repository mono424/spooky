export function nowMs(): number {
  return performance.now();
}

export async function timed<T>(fn: () => Promise<T>): Promise<{ value: T; ms: number }> {
  const start = nowMs();
  const value = await fn();
  return { value, ms: nowMs() - start };
}
