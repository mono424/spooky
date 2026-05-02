const startedAt = Date.now();

function ts(): string {
  const ms = Date.now() - startedAt;
  return `[+${(ms / 1000).toFixed(2)}s]`;
}

export const log = {
  info(msg: string, ...rest: unknown[]) {
    console.log(`${ts()} ${msg}`, ...rest);
  },
  warn(msg: string, ...rest: unknown[]) {
    console.warn(`${ts()} WARN ${msg}`, ...rest);
  },
  error(msg: string, ...rest: unknown[]) {
    console.error(`${ts()} ERR  ${msg}`, ...rest);
  },
  step(msg: string) {
    console.log(`\n${ts()} ── ${msg} ──`);
  },
};
