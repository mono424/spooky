import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

// Structural guard, in the spirit of sp00ky.init-query.test.ts: the paint path
// must stay network-free. `init()` resolves as soon as the LOCAL store is
// usable, and every consumer gates its first render on that, so anything that
// reaches the network belongs in `initRemote()` instead.
//
// This is the invariant that makes a warm reload instant and an offline boot
// possible at all. It is easy to undo by adding one innocent-looking `await`,
// hence a test rather than a comment.
const source = readFileSync(join(__dirname, 'sp00ky.ts'), 'utf8');

function methodBody(name: string): string {
  const re = new RegExp(`(?:private )?async ${name}\\([^)]*\\)[^{]*\\{[\\s\\S]*?\\n  \\}`);
  const m = re.exec(source);
  if (!m) throw new Error(`could not locate ${name}()`);
  return m[0];
}

describe('local-first boot', () => {
  const init = methodBody('init');

  it('init() never awaits the remote', () => {
    expect(init).not.toMatch(/await this\.remote\./);
    expect(init).not.toMatch(/await this\.auth\.init\(\)/);
  });

  it('init() hands the network half off without awaiting it', () => {
    expect(init).toMatch(/void this\.initRemote\(\)/);
  });

  it('init() restores the session locally and marks the client ready', () => {
    expect(init).toMatch(/await this\.auth\.restoreSessionFromToken\(\)/);
    expect(init).toMatch(/this\.localReady = true/);
  });

  it('the query-id salt is minted locally, not fetched from the server', () => {
    // No remaining CALL - the surviving mentions are comments explaining why.
    expect(source).not.toMatch(/query[^\n]*RETURN <string>session::id\(\)/);
    expect(init).toMatch(/this\.mintSessionSalt\(\)/);
  });

  it('permissions are seeded before anything can register a view', () => {
    const perms = init.indexOf('setPermissions(');
    const restore = init.indexOf('restoreSessionFromToken');
    expect(perms).toBeGreaterThan(-1);
    expect(perms).toBeLessThan(restore);
  });

  it('initRemote() tolerates an unreachable server and still supervises', () => {
    const remote = methodBody('initRemote');
    // connect is wrapped, not fatal
    expect(remote).toMatch(/try\s*\{[\s\S]*await this\.remote\.connect\(\)[\s\S]*\}\s*catch/);
    // and the supervisor starts regardless, or a boot-time failure never heals
    const connectCatchEnd = remote.indexOf('connectionSupervisor.start()');
    expect(connectCatchEnd).toBeGreaterThan(-1);
  });
});
