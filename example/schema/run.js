const fs = require('fs');
const path = require('path');
const { spawn } = require('child_process');

// Read arguments (e.g., "up -d", "down -v")
const args = process.argv.slice(2);

// Path to spooky.yml
const configPath = path.join(__dirname, 'spooky.yml');

// Default mode
let mode = 'singlenode';

// Try to read mode from spooky.yml
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

// Determine compose file
const composeFiles = {
  singlenode: 'docker-compose.singlenode.yml',
  cluster: 'docker-compose.cluster.yml',
  surrealism: 'docker-compose.surrealism.yml',
};
const composeFile = composeFiles[mode] || 'docker-compose.singlenode.yml';
console.log(`[box] Loading configuration from spooky.yml`);
console.log(`[box] Mode: ${mode}`);
console.log(`[box] Using: ${composeFile}`);

// Spawn docker-compose
const cmd = spawn('docker', ['compose', '-f', composeFile, ...args], { stdio: 'inherit' });

cmd.on('close', (code) => {
  process.exit(code);
});
