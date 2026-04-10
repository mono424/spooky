import { spawnSync } from 'child_process';
import { findBinary } from './resolve-binary.js';

const binary = findBinary();
const result = spawnSync(binary, process.argv.slice(2), { stdio: 'inherit' });

if (result.error) {
  console.error(`Failed to execute spky: ${result.error.message}`);
  process.exit(1);
}

process.exit(result.status ?? 1);
