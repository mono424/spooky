/**
 * Test-only helper: stubs `SqliteCacheEngine.createTransport` with a fake
 * transport driven by a plain handler function, replacing the old pattern of
 * faking a whole Worker plus the engine's pending-map wiring. Not exported
 * from the package; imported only by *.test.ts files.
 */
import type { SqliteTransport } from './sqlite-transport';

export type FakeTransportHandler = (type: string, payload: any) => unknown | Promise<unknown>;

/** Replace the engine's transport factory. Each open spawns a fresh fake, like
 *  the real factory spawns a fresh Worker. The handler returns the reply rest
 *  (without id/ok/wt) or throws to produce an error reply. */
export function stubTransport(engine: unknown, handler: FakeTransportHandler): void {
  (engine as { createTransport: () => SqliteTransport }).createTransport = () => {
    let closed = false;
    return {
      kind: 'worker',
      get connected() {
        return !closed;
      },
      call: <T>(type: string, payload?: unknown) =>
        Promise.resolve().then(() => handler(type, payload)) as Promise<T>,
      failAll() {},
      close() {
        closed = true;
      },
    } as SqliteTransport;
  };
}
