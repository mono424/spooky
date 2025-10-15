# @whitepawn/syncgen

Generate TypeScript and Dart types from SurrealDB schema files.

## Overview

This package wraps a Rust-based code generator that parses SurrealDB `.surql` schema files and generates type definitions. The core logic is implemented in Rust for performance, with a TypeScript/Node.js wrapper for easy integration into JavaScript/TypeScript projects.

## Installation

```bash
npm install @whitepawn/syncgen
```

## Prerequisites

- Rust toolchain (for building from source)
- Node.js 18+
- npx (for quicktype code generation)

## Usage

### CLI

```bash
# Generate TypeScript types
syncgen --input schema.surql --output types.ts

# Generate Dart types
syncgen --input schema.surql --output types.dart

# Generate JSON Schema
syncgen --input schema.surql --output schema.json

# Generate all formats at once
syncgen --input schema.surql --output output --all

# Specify format explicitly
syncgen --input schema.surql --output output.ts --format typescript
```

### Programmatic API

```typescript
import { runSyncgen } from '@whitepawn/syncgen';

// Generate types
const output = await runSyncgen({
  input: 'path/to/schema.surql',
  output: 'path/to/output.ts',
  format: 'typescript', // or 'dart', 'json'
  pretty: true,
  all: false
});

console.log(output);
```

## Options

- `--input, -i`: Path to the input `.surql` schema file (required)
- `--output, -o`: Path to the output file (required)
- `--format, -f`: Output format: `json`, `typescript`, `dart` (optional, inferred from extension)
- `--pretty, -p`: Pretty print JSON output (default: true)
- `--all, -a`: Generate all formats (TypeScript, Dart, and JSON Schema)

## Development

### Build

```bash
# Install dependencies
npm install

# Build Rust binary and TypeScript wrapper
npm run build

# Build only Rust
npm run build:rust

# Build only TypeScript
npm run build:vite
```

### Project Structure

```
syncgen/
├── src/
│   ├── main.rs          # Rust CLI entry point
│   ├── parser.rs        # SurrealDB schema parser
│   ├── json_schema.rs   # JSON Schema generator
│   ├── codegen.rs       # Code generation logic
│   ├── index.ts         # TypeScript wrapper
│   └── cli.ts           # TypeScript CLI wrapper
├── Cargo.toml           # Rust dependencies
├── package.json         # npm package config
├── vite.config.ts       # Vite build config
└── tsconfig.json        # TypeScript config
```

## How It Works

1. The Rust binary (`syncgen`) parses SurrealDB schema files and generates JSON Schema
2. For TypeScript/Dart output, it uses `quicktype` to convert JSON Schema to the target language
3. The TypeScript wrapper (`src/index.ts`) spawns the Rust binary as a child process
4. Vite bundles the TypeScript wrapper for distribution

## License

MIT
