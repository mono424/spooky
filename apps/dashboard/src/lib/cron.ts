/**
 * A plain-language reading of the cron shapes people actually write for
 * backups. Anything else is "custom expression": guessing wrong about a cron
 * line is worse than saying nothing, because the preview is what the operator
 * checks the field against.
 */

const DAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];

function clock(h: number, m: number): string {
  return `${h.toString().padStart(2, '0')}:${m.toString().padStart(2, '0')}`;
}

function dayName(d: string): string | null {
  const n = Number(d);
  if (Number.isInteger(n) && n >= 0 && n <= 7) return DAYS[n % 7]!;
  const idx = DAYS.findIndex((name) => name.slice(0, 3).toLowerCase() === d.toLowerCase());
  return idx >= 0 ? DAYS[idx]! : null;
}

export function describeCron(expr: string): string | null {
  const parts = expr.trim().split(/\s+/);
  if (parts.length !== 5) return null;
  const [min, hour, dom, mon, dow] = parts as [string, string, string, string, string];

  // */N * * * *
  if (/^\*\/\d+$/.test(min) && hour === '*' && dom === '*' && mon === '*' && dow === '*') {
    const n = Number(min.slice(2));
    return n === 1 ? 'every minute' : `every ${n} minutes`;
  }
  // M */N * * *
  if (/^\d+$/.test(min) && /^\*\/\d+$/.test(hour) && dom === '*' && mon === '*' && dow === '*') {
    const n = Number(hour.slice(2));
    return `every ${n} hour${n === 1 ? '' : 's'} at minute ${Number(min)}`;
  }
  // M * * * *
  if (/^\d+$/.test(min) && hour === '*' && dom === '*' && mon === '*' && dow === '*') {
    return `hourly at minute ${Number(min)}`;
  }
  // M H * * *
  if (/^\d+$/.test(min) && /^\d+$/.test(hour) && dom === '*' && mon === '*' && dow === '*') {
    return `daily at ${clock(Number(hour), Number(min))} UTC`;
  }
  // M H * * D[,D]
  if (/^\d+$/.test(min) && /^\d+$/.test(hour) && dom === '*' && mon === '*' && dow !== '*') {
    const names = dow.split(',').map(dayName);
    if (names.every((n) => n !== null)) {
      return `${names.join(', ')} at ${clock(Number(hour), Number(min))} UTC`;
    }
    if (dow === '1-5') return `weekdays at ${clock(Number(hour), Number(min))} UTC`;
  }
  // M H D * *
  if (/^\d+$/.test(min) && /^\d+$/.test(hour) && /^\d+$/.test(dom) && mon === '*' && dow === '*') {
    return `monthly on day ${Number(dom)} at ${clock(Number(hour), Number(min))} UTC`;
  }
  return null;
}
