import { spawn } from "child_process";
import { fileURLToPath } from "url";
import { dirname, join } from "path";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

export interface SyncgenOptions {
  input: string;
  output: string;
  format?: "json" | "typescript" | "dart";
  pretty?: boolean;
  all?: boolean;
  noHeader?: boolean;
}

export function runSyncgen(options: SyncgenOptions): Promise<string> {
  return new Promise((resolve, reject) => {
    const binaryPath = join(__dirname, "..", "target", "release", "syncgen");

    const args = ["--input", options.input, "--output", options.output];

    if (options.format) {
      args.push("--format", options.format);
    }

    if (options.pretty === true) {
      args.push("--pretty");
    }

    if (options.all === true) {
      args.push("--all");
    }

    if (options.noHeader === true) {
      args.push("--no-header");
    }

    const child = spawn(binaryPath, args);

    let stdout = "";
    let stderr = "";

    child.stdout.on("data", (data) => {
      stdout += data.toString();
    });

    child.stderr.on("data", (data) => {
      stderr += data.toString();
    });

    child.on("close", (code) => {
      if (code === 0) {
        resolve(stdout);
      } else {
        reject(new Error(`syncgen exited with code ${code}: ${stderr}`));
      }
    });

    child.on("error", (error) => {
      reject(error);
    });
  });
}
