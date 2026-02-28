const fs = require('fs');
const path = require('path');
const { spawn } = require('child_process');
const http = require('http');

// --- Configuration ---
const SPOOKY_BIN = path.resolve(__dirname, '../../apps/cli/target/release/spooky');
const HEALTH_URL = 'http://localhost:8666/health';
const MAX_RETRIES = 30;
const RETRY_INTERVAL_MS = 2000;
const MIGRATIONS_DIR = path.join(__dirname, 'migrations');

// Infrastructure services that must start before migrations
const INFRA_SERVICES = {
  singlenode: ['surrealdb', 'aspire-dashboard'],
  cluster: ['surrealdb'],
  surrealism: ['surrealdb'],
};

// --- Read mode from spooky.yml ---
const configPath = path.join(__dirname, 'spooky.yml');
let mode = 'singlenode';

if (fs.existsSync(configPath)) {
  const content = fs.readFileSync(configPath, 'utf8');
  if (content.match(/^mode:\s*"?cluster"?/m)) {
    mode = 'cluster';
  } else if (content.match(/^mode:\s*"?surrealism"?/m)) {
    mode = 'surrealism';
  } else if (content.match(/^mode:\s*"?singlenode"?/m)) {
    mode = 'singlenode';
  }
}

const composeFiles = {
  singlenode: 'docker-compose.singlenode.yml',
  cluster: 'docker-compose.cluster.yml',
  surrealism: 'docker-compose.surrealism.yml',
};
const composeFile = composeFiles[mode] || 'docker-compose.singlenode.yml';

console.log(`[box] Loading configuration from spooky.yml`);
console.log(`[box] Mode: ${mode}`);
console.log(`[box] Using: ${composeFile}`);

// --- Helpers ---

function runCommand(cmd, args, opts = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(cmd, args, { stdio: 'inherit', ...opts });
    child.on('close', (code) => {
      if (code !== 0) {
        reject(new Error(`"${cmd} ${args.join(' ')}" exited with code ${code}`));
      } else {
        resolve();
      }
    });
    child.on('error', reject);
  });
}

function waitForHealth(url, retries, intervalMs) {
  return new Promise((resolve, reject) => {
    let attempt = 0;
    const check = () => {
      attempt++;
      const req = http.get(url, (res) => {
        if (res.statusCode === 200) {
          console.log(`[box] SurrealDB is ready.`);
          resolve();
        } else {
          retry();
        }
      });
      req.on('error', () => retry());
      req.setTimeout(5000, () => {
        req.destroy();
        retry();
      });
    };
    const retry = () => {
      if (attempt >= retries) {
        reject(new Error(`SurrealDB did not become ready after ${retries} attempts.`));
      } else {
        console.log(`[box] Waiting for SurrealDB... (${attempt}/${retries})`);
        setTimeout(check, intervalMs);
      }
    };
    check();
  });
}

// --- Main ---

const args = process.argv.slice(2);
const subcommand = args[0]; // e.g. "up", "down", "stop", "logs"

async function main() {
  // For non-"up" commands, pass through directly
  if (subcommand !== 'up') {
    await runCommand('docker', ['compose', '-f', composeFile, ...args]);
    return;
  }

  // --- "up" command: multi-phase orchestration ---

  // 1. Check that the spooky CLI binary exists
  if (!fs.existsSync(SPOOKY_BIN)) {
    console.error(`[box] Error: spooky CLI binary not found at ${SPOOKY_BIN}`);
    console.error(`[box] Build it first: cd apps/cli && cargo build --release`);
    process.exit(1);
  }

  const infraServices = INFRA_SERVICES[mode] || INFRA_SERVICES.singlenode;

  // 2. Phase 1: Start infrastructure only (with all original flags)
  console.log(`\n[box] Phase 1: Starting infrastructure (${infraServices.join(', ')})...`);
  await runCommand('docker', ['compose', '-f', composeFile, ...args, ...infraServices]);

  // 3. Phase 2: Wait for SurrealDB to be healthy
  console.log(`\n[box] Phase 2: Waiting for SurrealDB health...`);
  await waitForHealth(HEALTH_URL, MAX_RETRIES, RETRY_INTERVAL_MS);

  // 4. Phase 3: Run migrations via spooky CLI
  console.log(`\n[box] Phase 3: Applying migrations...`);
  await runCommand(SPOOKY_BIN, [
    'migrate', 'apply',
    '--url', 'http://localhost:8666',
    '--migrations-dir', MIGRATIONS_DIR,
  ]);

  // 5. Phase 4: Start remaining services (without --force-recreate to avoid wiping DB)
  console.log(`\n[box] Phase 4: Starting remaining services...`);
  const phase4Args = args.filter((a) => a !== '--force-recreate');
  await runCommand('docker', ['compose', '-f', composeFile, ...phase4Args]);
}

main().catch((err) => {
  console.error(`[box] ${err.message}`);
  process.exit(1);
});
