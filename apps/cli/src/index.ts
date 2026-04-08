import { execFile } from 'child_process';
import { promisify } from 'util';
import { findBinary } from './resolve-binary.js';

const execFileAsync = promisify(execFile);

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
  endpoint?: string;
  secret?: string;
  config?: string;
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

  if (options.endpoint) {
    args.push('--endpoint', options.endpoint);
  }

  if (options.secret) {
    args.push('--secret', options.secret);
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
    throw new Error(`Syncgen failed: ${error.message}`, { cause: error });
  }
}
