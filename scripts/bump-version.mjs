import { execSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const ROOT = resolve(import.meta.dirname, "..");
const MANIFEST_PATH = join(ROOT, "apps/devtools/manifest.json");
const EXCLUDE = ["packages/ssp-wasm/pkg"];

const CLI_PLATFORM_PACKAGES = [
  "cli-linux-x64",
  "cli-linux-arm64",
  "cli-darwin-x64",
  "cli-darwin-arm64",
  "cli-win32-x64",
];

// ── helpers ──────────────────────────────────────────────────────────

function die(msg) {
  console.error(`Error: ${msg}`);
  process.exit(1);
}

function isValidSemver(v) {
  // Matches: 1.2.3, 1.2.3-canary.1, 1.2.3-beta.0, etc.
  return /^\d+\.\d+\.\d+(-[a-zA-Z0-9]+(\.[a-zA-Z0-9]+)*)?$/.test(v);
}

/** Convert semver to Chrome-compatible X.Y.Z.W format */
function toChromeVersion(semver) {
  const [core] = semver.split("-");
  const parts = core.split(".").map(Number);
  // Chrome requires exactly 4 dot-separated integers
  while (parts.length < 4) parts.push(0);
  return parts.slice(0, 4).join(".");
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf-8"));
}

function writeJson(path, data) {
  writeFileSync(path, JSON.stringify(data, null, 2) + "\n");
}

// ── discover workspace packages ──────────────────────────────────────

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

const version = process.argv[2];
if (!version) die("Usage: node scripts/bump-version.mjs <version>");
if (!isValidSemver(version)) die(`Invalid semver: "${version}"`);

const dirs = getWorkspacePackageDirs();
const updated = [];
const skipped = [];

// Bump every workspace package.json (including root)
for (const dir of dirs) {
  const pkgPath = join(dir, "package.json");
  const pkg = readJson(pkgPath);
  const rel = pkgPath.replace(ROOT + "/", "");

  if (pkg.version === version) {
    skipped.push(rel);
    continue;
  }

  const oldVersion = pkg.version;
  pkg.version = version;
  writeJson(pkgPath, pkg);
  updated.push(`  ${rel}: ${oldVersion} -> ${version}`);
}

// Bump CLI platform packages
for (const name of CLI_PLATFORM_PACKAGES) {
  const pkgPath = join(ROOT, "apps/cli/npm", name, "package.json");
  const pkg = readJson(pkgPath);
  const rel = pkgPath.replace(ROOT + "/", "");

  if (pkg.version === version) {
    skipped.push(rel);
  } else {
    const oldVersion = pkg.version;
    pkg.version = version;
    writeJson(pkgPath, pkg);
    updated.push(`  ${rel}: ${oldVersion} -> ${version}`);
  }
}

// Bump optionalDependencies in CLI package.json
const cliPkgPath = join(ROOT, "apps/cli/package.json");
const cliPkg = readJson(cliPkgPath);
if (cliPkg.optionalDependencies) {
  let cliUpdated = false;
  for (const dep of Object.keys(cliPkg.optionalDependencies)) {
    if (cliPkg.optionalDependencies[dep] !== version) {
      cliPkg.optionalDependencies[dep] = version;
      cliUpdated = true;
    }
  }
  if (cliUpdated) {
    writeJson(cliPkgPath, cliPkg);
    updated.push(`  apps/cli/package.json: optionalDependencies -> ${version}`);
  }
}

// Bump Chrome manifest
const manifest = readJson(MANIFEST_PATH);
const chromeVersion = toChromeVersion(version);
const manifestRel = MANIFEST_PATH.replace(ROOT + "/", "");

if (manifest.version === chromeVersion) {
  skipped.push(manifestRel);
} else {
  const oldManifestVersion = manifest.version;
  manifest.version = chromeVersion;
  writeJson(MANIFEST_PATH, manifest);
  updated.push(`  ${manifestRel}: ${oldManifestVersion} -> ${chromeVersion}`);
}

// Summary
if (updated.length === 0) {
  console.log(`Nothing to do — all files already at ${version}`);
} else {
  console.log(`Bumped ${updated.length} file(s) to ${version}:\n`);
  for (const line of updated) console.log(line);
  if (skipped.length > 0) {
    console.log(`\nSkipped ${skipped.length} file(s) (already at ${version})`);
  }
  console.log(`\nNext steps:`);
  console.log(`  git add -A && git commit -m "v${version}"`);
  console.log(`  git tag spooky/v${version}`);
  console.log(`  git push && git push --tags`);
}
