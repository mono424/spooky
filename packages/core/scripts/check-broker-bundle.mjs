// Build gate: dist/tabs-broker-worker.js must be self-contained ESM.
// Bundlers cannot trace `new SharedWorker(url)` module graphs, so any
// import statement surviving into the emitted broker file would 404 at
// runtime in consumers (the file is fetched as a bare URL, its relative
// chunk siblings are not copied along). Run after `tsdown` in the build.
import { readFileSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const dist = resolve(dirname(fileURLToPath(import.meta.url)), '../dist/tabs-broker-worker.js');

let code;
try {
  code = readFileSync(dist, 'utf8');
} catch {
  console.error(`check-broker-bundle: ${dist} missing; did tsdown run?`);
  process.exit(1);
}

const problems = [];
// Static or dynamic imports of sibling chunks.
const importRe = /(?:^|\n)\s*import[\s(]/;
if (importRe.test(code)) problems.push('contains an import statement');
if (!code.includes('onconnect')) problems.push('missing onconnect handler');

if (problems.length > 0) {
  console.error(`check-broker-bundle: dist/tabs-broker-worker.js ${problems.join('; ')}.`);
  console.error(
    'The broker worker must stay free of runtime imports (type-only imports are fine in source).'
  );
  process.exit(1);
}
console.log('check-broker-bundle: ok');
