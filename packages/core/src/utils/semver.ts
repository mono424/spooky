// Minimal semver comparison for app-release version checks. "X.Y.Z" numeric
// with missing parts read as 0 ("1.2" == "1.2.0"); any malformed input never
// compares greater, so a bad release row can never nag (or force-reload)
// every client.

function parse(v: unknown): [number, number, number] | null {
  const parts = String(v ?? '')
    .trim()
    .split('.');
  if (parts.length === 0 || parts.length > 3 || parts[0] === '') return null;
  const nums: number[] = [];
  for (let i = 0; i < 3; i++) {
    const raw = parts[i] ?? '0';
    if (!/^\d+$/.test(raw)) return null;
    nums.push(parseInt(raw, 10));
  }
  return nums as [number, number, number];
}

/** True when `a` is a valid version strictly greater than valid version `b`. */
export function semverGt(a: unknown, b: unknown): boolean {
  const pa = parse(a);
  const pb = parse(b);
  if (!pa || !pb) return false;
  for (let i = 0; i < 3; i++) {
    if (pa[i] > pb[i]) return true;
    if (pa[i] < pb[i]) return false;
  }
  return false;
}
