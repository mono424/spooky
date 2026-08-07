/**
 * Test-only fakes for the tabs broker: entangled MessagePort pairs with
 * microtask-async delivery (mimicking real ports), and a SharedWorker stand-in
 * that connects straight into the broker module's handleConnect. Imported only
 * by *.test.ts files.
 */

export class FakePort {
  other: FakePort | null = null;
  onmessage: ((ev: { data: any; ports: FakePort[] }) => void) | null = null;
  onmessageerror: ((ev?: unknown) => void) | null = null;
  closed = false;

  postMessage(data: any, transfer?: unknown[]): void {
    const target = this.other;
    if (!target || target.closed || this.closed) return;
    queueMicrotask(() => {
      if (!target.closed) target.onmessage?.({ data, ports: (transfer ?? []) as FakePort[] });
    });
  }

  start(): void {}

  close(): void {
    this.closed = true;
  }
}

export function fakeChannel(): { port1: FakePort; port2: FakePort } {
  const port1 = new FakePort();
  const port2 = new FakePort();
  port1.other = port2;
  port2.other = port1;
  return { port1, port2 };
}

/** Drain queued microtasks (message deliveries chain through several hops). */
export async function flush(times = 10): Promise<void> {
  for (let i = 0; i < times; i++) await Promise.resolve();
}

/**
 * Install a minimal exclusive-only `navigator.locks` on globalThis. Node has no
 * Web Locks, and `acquireLeaderTabLock` treats a missing LockManager as "always
 * granted", so without this the whole leader-tab-lock path is a no-op in tests
 * and lock leaks are invisible. `ifAvailable` resolves null while held (what a
 * losing tab sees), `steal` evicts the holder, and a plain request queues
 * forever — the same shapes the real API produces.
 */
export function installFakeLocks(): {
  restore: () => void;
  heldNames: () => string[];
} {
  const g = globalThis as Record<string, unknown>;
  const previous = g.navigator;
  const held = new Map<string, () => void>();
  const locks = {
    async request(name: string, opts: any, cb: any) {
      if (opts?.steal) held.get(name)?.();
      if (held.has(name) && !opts?.steal) {
        if (opts?.ifAvailable) return cb(null);
        return new Promise(() => {});
      }
      // The holder keeps the lock until the callback's promise settles (the
      // real contract) or someone steals it out from under them.
      let stolen!: () => void;
      const stealSignal = new Promise<void>((r) => {
        stolen = r as () => void;
      });
      held.set(name, stolen);
      try {
        await Promise.race([Promise.resolve(cb({ name, mode: 'exclusive' })), stealSignal]);
      } finally {
        held.delete(name);
      }
    },
  };
  const define = (value: unknown) =>
    Object.defineProperty(g, 'navigator', { value, configurable: true, writable: true });
  define({ ...(previous ?? {}), locks });
  return { restore: () => define(previous), heldNames: () => [...held.keys()] };
}

/** Install `MessageChannel` + `SharedWorker` fakes on globalThis; the fake
 *  SharedWorker pipes its port into `handleConnect`. Returns a restore fn. */
export function installBrokerGlobals(handleConnect: (port: MessagePort) => void): () => void {
  const g = globalThis as Record<string, unknown>;
  const prevMC = g.MessageChannel;
  const prevSW = g.SharedWorker;
  g.MessageChannel = class {
    port1: FakePort;
    port2: FakePort;
    constructor() {
      const { port1, port2 } = fakeChannel();
      this.port1 = port1;
      this.port2 = port2;
    }
  };
  g.SharedWorker = class {
    port: FakePort;
    onerror: ((e: unknown) => void) | null = null;
    constructor() {
      const { port1, port2 } = fakeChannel();
      this.port = port2;
      handleConnect(port1 as unknown as MessagePort);
    }
  };
  return () => {
    g.MessageChannel = prevMC;
    g.SharedWorker = prevSW;
  };
}
