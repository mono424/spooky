import { Command } from "commander";
import { runSyncgen } from "./index.js";

const program = new Command();

program
  .name("syncgen")
  .description("Generate types from SurrealDB schema files")
  .version("0.1.0");

program
  .option("-i, --input <path>", "Path to the input .surql schema file")
  .option(
    "-o, --output <path>",
    "Path to the output file (extension determines format: .json, .ts, .dart)"
  )
  .option("-f, --format <format>", "Output format (json, typescript, dart)")
  .option(
    "-p, --pretty",
    "Pretty print the JSON output (only for JSON format)",
    true
  )
  .option(
    "-a, --all",
    "Generate all formats (TypeScript and Dart) in addition to JSON Schema",
    false
  )
  .option(
    "--no-header",
    "Disable the generated file comment header (enabled by default)",
    false
  )
  .parse(process.argv);

const options = program.opts();

async function main() {
  if (!options.input || !options.output) {
    console.error("Error: --input and --output are required");
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
    });

    console.log(output);
  } catch (error) {
    console.error(
      "Error:",
      error instanceof Error ? error.message : String(error)
    );
    process.exit(1);
  }
}

main();
