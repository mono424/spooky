import { execSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";

const ROOT = resolve(import.meta.dirname, "..");
const EXCLUDE = ["packages/ssp-wasm/pkg"];

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf-8"));
}

function getWorkspacePackageDirs() {
  const raw = execSync("pnpm -r list --json --depth -1", {
    cwd: ROOT,
    encoding: "utf-8",
  });
  const packages = JSON.parse(raw);
  return packages
    .map((pkg) => pkg.path)
    .filter((p) => !EXCLUDE.some((ex) => p.endsWith(ex)));
}

// ── main ─────────────────────────────────────────────────────────────

const expected = process.argv[2];
if (!expected) {
  console.error("Usage: node scripts/check-version.mjs <expected-version>");
  process.exit(1);
}

const dirs = getWorkspacePackageDirs();
const mismatches = [];

for (const dir of dirs) {
  const pkgPath = join(dir, "package.json");
  const pkg = readJson(pkgPath);
  const rel = pkgPath.replace(ROOT + "/", "");

  if (pkg.version !== expected) {
    mismatches.push({ file: rel, actual: pkg.version });
  }
}

if (mismatches.length > 0) {
  console.error(`Version mismatch! Expected ${expected} but found:\n`);
  for (const m of mismatches) {
    console.error(`  ${m.file}: ${m.actual}`);
  }
  console.error(`\nRun: pnpm version:bump ${expected}`);
  process.exit(1);
}

console.log(`All ${dirs.length} workspace packages are at ${expected}`);
