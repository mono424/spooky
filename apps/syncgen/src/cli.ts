import { Command } from 'commander';
import { runSyncgen } from './index.js';

const program = new Command();

program.name('syncgen').description('Generate types from SurrealDB schema files').version('0.1.0');

program
  .option('-i, --input <path>', 'Path to the input .surql schema file')
  .option(
    '-o, --output <path>',
    'Path to the output file (extension determines format: .json, .ts, .dart)'
  )
  .option('-f, --format <format>', 'Output format (json, typescript, dart, surql)')
  .option('-p, --pretty', 'Pretty print the JSON output (only for JSON format)', true)
  .option(
    '-a, --all',
    'Generate all formats (TypeScript and Dart) in addition to JSON Schema',
    false
  )
  .option('--no-header', 'Disable the generated file comment header (enabled by default)', false)
  .option('--append <path>', 'Path to another .surql file to append to the input')
  .option('--modules-dir <path>', 'Directory containing Surrealism modules to compile and bundle')
  .option(
    '--mode <mode>',
    'Generation mode: "surrealism" (embedded WASM) or "sidecar" (HTTP calls)',
    'surrealism'
  )
  .option('--sidecar-endpoint <url>', 'Spooky Sidecar Endpoint URL')
  .option('--sidecar-secret <secret>', 'Spooky Sidecar Auth Secret')
  .parse(process.argv);

const options = program.opts();

async function main() {
  if (!options.input || !options.output) {
    console.error('Error: --input and --output are required');
    program.help();
    process.exit(1);
  }

  try {
    const output = await runSyncgen({
      input: options.input,
      output: options.output,
      format: options.format,
      pretty: options.pretty,
      all: options.all,
      noHeader: options.noHeader,
      append: options.append,
      modulesDir: options.modulesDir,
      mode: options.mode,
      sidecarEndpoint: options.sidecarEndpoint,
      sidecarSecret: options.sidecarSecret,
    });

    console.log(output);
  } catch (error) {
    console.error('Error:', error instanceof Error ? error.message : String(error));
    process.exit(1);
  }
}

main();
