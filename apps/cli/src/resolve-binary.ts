import { platform, arch } from 'os';
import { resolve, dirname } from 'path';
import { existsSync } from 'fs';
import { fileURLToPath } from 'url';
import { createRequire } from 'module';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const PLATFORM_PACKAGES: Record<string, string> = {
  'darwin-arm64': '@spooky-sync/cli-darwin-arm64',
  'darwin-x64': '@spooky-sync/cli-darwin-x64',
  'linux-arm64': '@spooky-sync/cli-linux-arm64',
  'linux-x64': '@spooky-sync/cli-linux-x64',
  'win32-x64': '@spooky-sync/cli-win32-x64',
};

function getPlatformBinary(): string | undefined {
  const key = `${platform()}-${arch()}`;
  const pkg = PLATFORM_PACKAGES[key];
  if (!pkg) return undefined;

  const binaryName = platform() === 'win32' ? 'spky.exe' : 'spky';

  try {
    const require = createRequire(import.meta.url);
    const pkgJson = require.resolve(`${pkg}/package.json`);
    return resolve(dirname(pkgJson), binaryName);
  } catch {
    return undefined;
  }
}

export function findBinary(): string {
  const binaryName = platform() === 'win32' ? 'spky.exe' : 'spky';

  // 1. Platform-specific npm package
  const platformBinary = getPlatformBinary();
  if (platformBinary && existsSync(platformBinary)) {
    return platformBinary;
  }

  // 2. Local dev build (from dist/ -> ../target/release/ or ../target/debug/)
  const releasePath = resolve(__dirname, '../target/release', binaryName);
  if (existsSync(releasePath)) {
    return releasePath;
  }

  const debugPath = resolve(__dirname, '../target/debug', binaryName);
  if (existsSync(debugPath)) {
    return debugPath;
  }

  // 3. Legacy fallback (binary next to dist/)
  const legacyPath = resolve(__dirname, '..', binaryName);
  if (existsSync(legacyPath)) {
    return legacyPath;
  }

  // 4. CWD fallback
  const cwdPath = resolve(process.cwd(), binaryName);
  if (existsSync(cwdPath)) {
    return cwdPath;
  }

  const key = `${platform()}-${arch()}`;
  const pkg = PLATFORM_PACKAGES[key];
  const hint = pkg
    ? `\nTry installing the platform package: npm install ${pkg}`
    : `\nYour platform (${key}) is not supported.`;

  throw new Error(
    `Could not find spky binary. Checked paths:\n` +
      `  - Platform package (${pkg ?? 'none'})\n` +
      `  - ${releasePath}\n` +
      `  - ${debugPath}\n` +
      `  - ${legacyPath}\n` +
      `  - ${cwdPath}\n` +
      hint
  );
}
