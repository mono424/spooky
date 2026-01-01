const fs = require('fs');
const path = require('path');
const { spawn } = require('child_process');

// Read arguments (e.g., "up -d", "down -v")
const args = process.argv.slice(2);

// Path to spooky.yml
const configPath = path.join(__dirname, 'spooky.yml');

// Default mode
let mode = 'surrealism';

// Try to read mode from spooky.yml
if (fs.existsSync(configPath)) {
  const content = fs.readFileSync(configPath, 'utf8');
  // Simple check for "mode: sidecar"
  if (content.match(/^mode:\s*"?sidecar"?/m)) {
    mode = 'sidecar';
  }
}

// Determine compose file
const composeFile = mode === 'sidecar' ? 'docker-compose.sidecar.yml' : 'docker-compose.surrealism.yml';
console.log(`[box] Loading configuration from spooky.yml`);
console.log(`[box] Mode: ${mode}`);
console.log(`[box] Using: ${composeFile}`);

// Spawn docker-compose
const cmd = spawn('docker', ['compose', '-f', composeFile, ...args], { stdio: 'inherit' });

cmd.on('close', (code) => {
  process.exit(code);
});
