import { execFile } from 'child_process';
import { promisify } from 'util';
import { resolve, dirname } from 'path';
import { existsSync } from 'fs';
import { platform } from 'os';
import { fileURLToPath } from 'url';

const execFileAsync = promisify(execFile);

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

export interface SyncgenOptions {
  input: string;
  output: string;
  format?: 'json' | 'typescript' | 'dart' | 'surql';
  pretty?: boolean;
  all?: boolean;
  noHeader?: boolean;
  append?: string;
  modulesDir?: string;
  mode?: string;
  sidecarEndpoint?: string;
  sidecarSecret?: string;
  config?: string;
}

function findBinary(): string {
  // Possible locations for the binary
  const binaryName = platform() === 'win32' ? 'spooky.exe' : 'spooky';

  const possiblePaths = [
    // Development/Source build location (from dist/index.js -> ../target/release)
    resolve(__dirname, '../target/release', binaryName),
    // Distributed package location (relative to dist/index.js -> ../bin)
    resolve(__dirname, '..', binaryName),
    // Installation root
    resolve(process.cwd(), binaryName),
  ];

  for (const path of possiblePaths) {
    if (existsSync(path)) {
      return path;
    }
  }

  throw new Error(
    `Could not find syncgen binary. Checked paths:\n${possiblePaths.map((p) => ` - ${p}`).join('\n')}\nPlease ensure it is built or installed.`
  );
}

export async function runSyncgen(options: SyncgenOptions): Promise<string> {
  const binaryPath = findBinary();
  const args: string[] = [];

  // Required arguments
  args.push('--input', options.input);
  args.push('--output', options.output);

  // Optional arguments
  if (options.format) {
    args.push('--format', options.format);
  }

  if (options.pretty) {
    args.push('--pretty');
  }

  if (options.all) {
    args.push('--all');
  }

  if (options.noHeader) {
    args.push('--no-header');
  }

  if (options.append) {
    args.push('--append', options.append);
  }

  if (options.modulesDir) {
    args.push('--modules-dir', options.modulesDir);
  }

  if (options.mode) {
    args.push('--mode', options.mode);
  }

  if (options.sidecarEndpoint) {
    args.push('--sidecar-endpoint', options.sidecarEndpoint);
  }

  if (options.sidecarSecret) {
    args.push('--sidecar-secret', options.sidecarSecret);
  }

  if (options.config) {
    args.push('--config', options.config);
  }

  try {
    const { stdout, stderr } = await execFileAsync(binaryPath, args);
    if (stderr) {
      console.error(stderr);
    }
    return stdout;
  } catch (error: any) {
    throw new Error(`Syncgen failed: ${error.message}`);
  }
}
