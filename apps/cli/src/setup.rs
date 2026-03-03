use anyhow::{Context, Result};
use inquire::Select;
use inquire::Text;
use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// Constants – large string literals extracted for readability
// ---------------------------------------------------------------------------

const MINIMAL_SCHEMA: &str = r#"-- ##################################################################
-- SCOPES & AUTHENTICATION
-- ##################################################################

DEFINE ACCESS account ON DATABASE TYPE RECORD
  SIGNUP ( CREATE user SET username = $username, password = crypto::argon2::generate($password) )
  SIGNIN ( SELECT * FROM user WHERE username = $username AND crypto::argon2::compare(password, $password) )
  DURATION FOR TOKEN 15m, FOR SESSION 30d
;

-- ##################################################################
-- USER TABLE
-- ##################################################################

DEFINE TABLE user SCHEMAFULL
PERMISSIONS
  FOR update, delete WHERE $access = "account" AND id = $auth.id
  FOR create, select WHERE true;

DEFINE FIELD username ON TABLE user TYPE string
ASSERT $value != NONE AND string::len($value) > 3
PERMISSIONS
    FOR select WHERE true
    FOR create WHERE true
    FOR update WHERE $access = "account" AND id = $auth.id;

DEFINE INDEX unique_username ON TABLE user FIELDS username UNIQUE;

DEFINE FIELD password ON TABLE user TYPE string
ASSERT $value != NONE AND string::len($value) > 0
PERMISSIONS
    FOR select WHERE false
    FOR create WHERE true
    FOR update WHERE $access = "account" AND id = $auth.id;

DEFINE FIELD created_at ON TABLE user TYPE datetime
VALUE time::now()
PERMISSIONS
    FOR select WHERE false
    FOR create WHERE true
    FOR update WHERE $access = "account" AND id = $auth.id;

DEFINE FIELD profile_picture ON TABLE user TYPE option<string>
PERMISSIONS
    FOR select WHERE true
    FOR create, update WHERE $access = "account" AND id = $auth.id;
"#;

const RUN_JS: &str = r#"const fs = require('fs');
const path = require('path');
const { spawn } = require('child_process');
const http = require('http');

// --- Configuration ---
const HEALTH_URL = 'http://localhost:8666/health';
const MAX_RETRIES = 30;
const RETRY_INTERVAL_MS = 2000;
const MIGRATIONS_DIR = path.join(__dirname, 'migrations');

// Infrastructure services that must start before migrations
const INFRA_SERVICES = {
  singlenode: ['surrealdb', 'aspire-dashboard'],
  cluster: ['surrealdb'],
  surrealism: ['surrealdb'],
};

// --- Read mode from spooky.yml ---
const configPath = path.join(__dirname, 'spooky.yml');
let mode = 'singlenode';

if (fs.existsSync(configPath)) {
  const content = fs.readFileSync(configPath, 'utf8');
  if (content.match(/^mode:\s*"?cluster"?/m)) {
    mode = 'cluster';
  } else if (content.match(/^mode:\s*"?surrealism"?/m)) {
    mode = 'surrealism';
  } else if (content.match(/^mode:\s*"?singlenode"?/m)) {
    mode = 'singlenode';
  }
}

const composeFiles = {
  singlenode: 'docker-compose.singlenode.yml',
  cluster: 'docker-compose.cluster.yml',
  surrealism: 'docker-compose.surrealism.yml',
};
const composeFile = composeFiles[mode] || 'docker-compose.singlenode.yml';

console.log(`[box] Loading configuration from spooky.yml`);
console.log(`[box] Mode: ${mode}`);
console.log(`[box] Using: ${composeFile}`);

// --- Helpers ---

function runCommand(cmd, args, opts = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(cmd, args, { stdio: 'inherit', ...opts });
    child.on('close', (code) => {
      if (code !== 0) {
        reject(new Error(`"${cmd} ${args.join(' ')}" exited with code ${code}`));
      } else {
        resolve();
      }
    });
    child.on('error', reject);
  });
}

function runSpooky(args, opts = {}) {
  const localBin = path.resolve(__dirname, 'node_modules', '.bin', 'spooky');
  if (fs.existsSync(localBin)) return runCommand(localBin, args, opts);
  return runCommand('npx', ['spooky', ...args], opts);
}

function waitForHealth(url, retries, intervalMs) {
  return new Promise((resolve, reject) => {
    let attempt = 0;
    let done = false;
    const check = () => {
      if (done) return;
      attempt++;
      const req = http.get(url, (res) => {
        if (done) return;
        if (res.statusCode === 200) {
          done = true;
          console.log(`[box] SurrealDB is ready.`);
          resolve();
        } else {
          retry();
        }
      });
      req.on('error', () => { if (!done) retry(); });
      req.setTimeout(5000, () => {
        req.destroy();
      });
    };
    const retry = () => {
      if (done) return;
      if (attempt >= retries) {
        done = true;
        reject(new Error(`SurrealDB did not become ready after ${retries} attempts.`));
      } else {
        console.log(`[box] Waiting for SurrealDB... (${attempt}/${retries})`);
        setTimeout(check, intervalMs);
      }
    };
    check();
  });
}

// --- Main ---

const args = process.argv.slice(2);
const subcommand = args[0]; // e.g. "up", "down", "stop", "logs"

async function main() {
  // For non-"up" commands, pass through directly
  if (subcommand !== 'up') {
    const orphanFlag = (subcommand === 'down') ? ['--remove-orphans'] : [];
    await runCommand('docker', ['compose', '-f', composeFile, subcommand, ...orphanFlag, ...args.slice(1)]);
    return;
  }

  // --- "up" command: multi-phase orchestration ---

  const infraServices = INFRA_SERVICES[mode] || INFRA_SERVICES.singlenode;

  // 1. Phase 1: Start infrastructure only (always detached so we can proceed to health check)
  const upFlags = args.slice(1).filter((a) => a !== '-d');
  console.log(`\n[box] Phase 1: Starting infrastructure (${infraServices.join(', ')})...`);
  await runCommand('docker', ['compose', '-f', composeFile, 'up', '-d', '--remove-orphans', ...upFlags, ...infraServices]);

  // 2. Phase 2: Wait for SurrealDB to be healthy
  console.log(`\n[box] Phase 2: Waiting for SurrealDB health...`);
  await waitForHealth(HEALTH_URL, MAX_RETRIES, RETRY_INTERVAL_MS);

  // 3. Phase 3: Run migrations via spooky CLI
  console.log(`\n[box] Phase 3: Applying migrations...`);
  await runSpooky([
    'migrate', 'apply',
    '--url', 'http://localhost:8666',
    '--migrations-dir', MIGRATIONS_DIR,
  ]);

  // 4. Phase 4: Start remaining services (without --force-recreate to avoid wiping DB)
  console.log(`\n[box] Phase 4: Starting remaining services...`);
  const phase4Args = args.filter((a) => a !== '--force-recreate');
  await runCommand('docker', ['compose', '-f', composeFile, 'up', '--remove-orphans', ...phase4Args.slice(1)]);
}

main().catch((err) => {
  console.error(`[box] ${err.message}`);
  process.exit(1);
});
"#;

const DOCKER_COMPOSE_SINGLENODE: &str = r#"services:
  aspire-dashboard:
    image: mcr.microsoft.com/dotnet/aspire-dashboard:latest
    container_name: aspire-dashboard
    environment:
      - DOTNET_DASHBOARD_UNSECURED_ALLOW_ANONYMOUS=true
      - DOTNET_DASHBOARD_OTLP_AUTH_MODE=Unsecured
      - ASPNETCORE_ENVIRONMENT=Development
    ports:
      - '18888:18888'
      - '4317:18889'
      - '4318:18890'
    restart: unless-stopped

  surrealdb:
    image: surrealdb/surrealdb:v3.0.0
    ports:
      - '8666:8000'
    environment:
      - SURREAL_USER=root
      - SURREAL_PASS=root
      - SURREAL_LOG=info
    command:
      - start
      - --bind
      - 0.0.0.0:8000
      - --allow-all
      - --user
      - root
      - --pass
      - root
      - memory
    depends_on:
      - aspire-dashboard
    healthcheck:
      test: ['CMD', '/surreal', 'is-ready', '--endpoint', 'http://localhost:8000']
      interval: 10s
      timeout: 5s
      retries: 5
    restart: unless-stopped

  ssp:
    image: mono424/spooky-ssp:latest
    ports:
      - '8667:8667'
    environment:
      - RUST_LOG=info,ssp=debug
      - OTEL_EXPORTER_OTLP_ENDPOINT=http://aspire-dashboard:18889
      - OTEL_SERVICE_NAME=spooky-ssp
      - SURREALDB_ADDR=surrealdb:8000/rpc
      - SURREALDB_NS=main
      - SURREALDB_DB=main
      - SURREALDB_USER=root
      - SURREALDB_PASS=root
      - SPOOKY_AUTH_SECRET=mysecret
      - SPOOKY_PERSISTENCE_FILE=/data/spooky_state.json
      - SPOOKY_CONFIG_PATH=/config/spooky.yml
    volumes:
      - ./.spooky/ssp_data:/data
      - ./spooky.yml:/config/spooky.yml:ro
    depends_on:
      surrealdb:
        condition: service_healthy
      aspire-dashboard:
        condition: service_started
    restart: unless-stopped
"#;

const DOCKER_COMPOSE_CLUSTER: &str = r#"x-ssp-common: &ssp-common
  image: mono424/spooky-ssp:latest
  environment: &ssp-env
    RUST_LOG: "info,ssp=debug"
    OTEL_EXPORTER_OTLP_ENDPOINT: "http://aspire-dashboard:18889"
    OTEL_SERVICE_NAME: "spooky-ssp"
    SURREALDB_ADDR: "surrealdb:8000/rpc"
    SURREALDB_NS: "main"
    SURREALDB_DB: "main"
    SURREALDB_USER: "root"
    SURREALDB_PASS: "root"
    SPOOKY_AUTH_SECRET: "mysecret"
    SPOOKY_PERSISTENCE_FILE: "/data/spooky_state.json"
    SPOOKY_CONFIG_PATH: "/config/spooky.yml"
    SCHEDULER_URL: "http://scheduler:9667"
  volumes:
    - ./spooky.yml:/config/spooky.yml:ro
  depends_on:
    scheduler:
      condition: service_healthy
  restart: unless-stopped

services:
  aspire-dashboard:
    image: mcr.microsoft.com/dotnet/aspire-dashboard:latest
    profiles:
      - observability
    container_name: aspire-dashboard
    environment:
      - DOTNET_DASHBOARD_UNSECURED_ALLOW_ANONYMOUS=true
      - DOTNET_DASHBOARD_OTLP_AUTH_MODE=Unsecured
      - ASPNETCORE_ENVIRONMENT=Development
    ports:
      - '18888:18888'
      - '4317:18889'
      - '4318:18890'
    restart: unless-stopped

  surrealdb:
    image: surrealdb/surrealdb:v3.0.0
    ports:
      - '8666:8000'
    environment:
      - SURREAL_USER=root
      - SURREAL_PASS=root
      - SURREAL_LOG=info
    command:
      - start
      - --bind
      - 0.0.0.0:8000
      - --allow-all
      - --user
      - root
      - --pass
      - root
      - memory
    healthcheck:
      test: ['CMD', '/surreal', 'is-ready', '--endpoint', 'http://localhost:8000']
      interval: 10s
      timeout: 5s
      retries: 5
    restart: unless-stopped

  scheduler:
    image: mono424/spooky-scheduler:latest
    ports:
      - '9667:9667'
    environment:
      - RUST_LOG=info,scheduler=debug
      - SPOOKY_SCHEDULER_DB_URL=surrealdb:8000/rpc
      - SPOOKY_SCHEDULER_DB_NAMESPACE=main
      - SPOOKY_SCHEDULER_DB_DATABASE=main
      - SPOOKY_SCHEDULER_DB_USERNAME=root
      - SPOOKY_SCHEDULER_DB_PASSWORD=root
      - SPOOKY_SCHEDULER_REPLICA_DB_PATH=/data/replica
      - SPOOKY_SCHEDULER_WAL_PATH=/data/event_wal.log
      - SPOOKY_AUTH_SECRET=mysecret
    volumes:
      - scheduler-data:/data
    depends_on:
      surrealdb:
        condition: service_healthy
    healthcheck:
      test: ['CMD-SHELL', 'curl -sf http://localhost:9667/metrics || exit 1']
      interval: 10s
      timeout: 5s
      retries: 5
      start_period: 30s
    restart: unless-stopped

  ssp-1:
    <<: *ssp-common
    environment:
      <<: *ssp-env
      SSP_ID: "ssp-1"
      ADVERTISE_ADDR: "ssp-1:8667"

  ssp-2:
    <<: *ssp-common
    environment:
      <<: *ssp-env
      SSP_ID: "ssp-2"
      ADVERTISE_ADDR: "ssp-2:8667"

  ssp-3:
    <<: *ssp-common
    environment:
      <<: *ssp-env
      SSP_ID: "ssp-3"
      ADVERTISE_ADDR: "ssp-3:8667"

volumes:
  scheduler-data:
"#;

const GITIGNORE: &str = "node_modules/
.spooky/ssp_data/
.spooky/*.gen.surql
.spooky/*.surli
";

const SPOOKY_YML: &str = "mode: singlenode
buckets: []
";

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn setup_project() -> Result<()> {
    println!("Welcome to Spooky Project Setup! 👻");

    // 1. Project name
    let project_name = Text::new("What is the name of your project?").prompt()?;

    // 2. Project kind
    let project_kind = Select::new(
        "What kind of project?",
        vec!["Schema only", "Full project (Schema + App)"],
    )
    .prompt()?;
    let is_schema_only = project_kind == "Schema only";

    // 3. Schema template
    let schema_type = Select::new(
        "Which schema template?",
        vec![
            "Empty",
            "Minimal (User + Auth)",
            "Example (User + Threads + Comments)",
        ],
    )
    .prompt()?;

    // 4. Codegen format
    let codegen_format = if is_schema_only {
        let fmt = Select::new(
            "Which output format for code generation?",
            vec!["TypeScript", "Dart", "Skip"],
        )
        .prompt()?;
        fmt.to_string()
    } else {
        "TypeScript".to_string() // full project always uses TypeScript
    };

    println!(
        "\nSetting up {} project '{}' with {} schema...",
        if is_schema_only { "schema-only" } else { "full" },
        project_name,
        schema_type
    );

    // --- Create root directory ---
    let root_path = Path::new(&project_name);
    if root_path.exists() {
        if !inquire::Confirm::new("Directory already exists. Overwrite?")
            .with_default(false)
            .prompt()?
        {
            println!("Aborting setup.");
            return Ok(());
        }
        fs::remove_dir_all(root_path)?;
    }
    fs::create_dir_all(root_path)?;

    // --- Determine schema package path ---
    let schema_path = if is_schema_only {
        root_path.to_path_buf()
    } else {
        root_path.join("packages/schema")
    };

    // --- Write schema package ---
    write_schema_package(
        &schema_path,
        &project_name,
        schema_type,
        &codegen_format,
        is_schema_only,
    )?;

    if is_schema_only {
        // Schema-only: root IS the schema package, .gitignore at root
        write_file(root_path.join(".gitignore"), GITIGNORE)?;
    } else {
        // Full project: monorepo structure
        write_file(root_path.join(".gitignore"), GITIGNORE)?;

        write_file(
            root_path.join("pnpm-workspace.yaml"),
            "packages:\n  - 'apps/*'\n  - 'packages/*'\n",
        )?;

        write_file(
            root_path.join("package.json"),
            &format!(
                r#"{{
  "name": "{}",
  "private": true,
  "scripts": {{
    "dev": "pnpm -r dev",
    "build": "pnpm -r build"
  }}
}}"#,
                project_name
            ),
        )?;

        // --- App (SolidJS) setup ---
        write_app_package(root_path)?;
    }

    // --- Success message ---
    println!("\n✓ Project '{}' created successfully!", project_name);
    println!("\nNext steps:");

    if is_schema_only {
        println!("  cd {}", project_name);
        println!("  pnpm install");
        println!("  pnpm start");
    } else {
        println!("  cd {}", project_name);
        println!("  pnpm install");
        println!("  cd packages/schema && pnpm start");
        println!("  cd ../../apps/web && pnpm dev");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Schema package creation
// ---------------------------------------------------------------------------

fn write_schema_package(
    schema_path: &Path,
    project_name: &str,
    schema_type: &str,
    codegen_format: &str,
    is_schema_only: bool,
) -> Result<()> {
    // Create directories
    fs::create_dir_all(schema_path.join("src/buckets"))?;
    fs::create_dir_all(schema_path.join("src/outbox"))?;
    fs::create_dir_all(schema_path.join("migrations"))?;
    fs::create_dir_all(schema_path.join(".spooky"))?;

    // Schema content
    let schema_content = match schema_type {
        "Empty" => "-- Empty Schema\n".to_string(),
        "Minimal (User + Auth)" => MINIMAL_SCHEMA.to_string(),
        "Example (User + Threads + Comments)" => {
            include_str!("../../../example/schema/src/schema.surql").to_string()
        }
        _ => "-- Empty Schema\n".to_string(),
    };
    write_file(schema_path.join("src/schema.surql"), &schema_content)?;

    // package.json
    let package_json = build_schema_package_json(project_name, codegen_format, is_schema_only);
    write_file(schema_path.join("package.json"), &package_json)?;

    // spooky.yml
    write_file(schema_path.join("spooky.yml"), SPOOKY_YML)?;

    // run.js
    write_file(schema_path.join("run.js"), RUN_JS)?;

    // Docker compose files
    write_file(
        schema_path.join("docker-compose.singlenode.yml"),
        DOCKER_COMPOSE_SINGLENODE,
    )?;
    write_file(
        schema_path.join("docker-compose.cluster.yml"),
        DOCKER_COMPOSE_CLUSTER,
    )?;
    write_file(
        schema_path.join("docker-compose.surrealism.yml"),
        include_str!("../../../example/schema/docker-compose.surrealism.yml"),
    )?;

    // .gitignore (only when nested inside a monorepo — root .gitignore is written by caller)
    if !is_schema_only {
        write_file(schema_path.join(".gitignore"), GITIGNORE)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// package.json builder for the schema package
// ---------------------------------------------------------------------------

fn build_schema_package_json(
    project_name: &str,
    codegen_format: &str,
    is_schema_only: bool,
) -> String {
    let pkg_name = if is_schema_only {
        project_name.to_string()
    } else {
        format!("@{}/schema", project_name)
    };

    let codegen_ext = match codegen_format {
        "TypeScript" => "ts",
        "Dart" => "dart",
        _ => "",
    };

    let format_flag = codegen_format.to_lowercase();

    // build:ts script — output path differs for schema-only vs monorepo
    let build_ts_output = if is_schema_only {
        format!("./schema.gen.{}", codegen_ext)
    } else {
        format!("../../apps/web/src/schema.gen.{}", codegen_ext)
    };

    let build_ts_script = format!(
        "spooky --format {} --input ./src/schema.surql --output {} --config ./spooky.yml",
        format_flag, build_ts_output
    );

    let (build_script, extra_scripts) = if codegen_format == "Skip" {
        // No codegen — build only produces surql
        (
            "\"build\": \"pnpm build:surql\"".to_string(),
            String::new(),
        )
    } else {
        (
            "\"build\": \"pnpm build:ts && pnpm build:surql\"".to_string(),
            format!(
                ",\n    \"build:ts\": \"{}\"",
                build_ts_script
            ),
        )
    };

    format!(
        r#"{{
  "name": "{}",
  "version": "1.0.0",
  "scripts": {{
    "start": "node run.js up -d",
    "stop": "node run.js down",
    "dev": "pnpm build && node run.js up --build --force-recreate",
    "db:reset": "rm -rf .spooky/ssp_data && mkdir .spooky/ssp_data && node run.js down -v",
    {}{},
    "build:surql": "spooky --input ./src/schema.surql --output ./.spooky/remote-singlenode.gen.surql --mode singlenode --config spooky.yml"
  }},
  "devDependencies": {{
    "@spooky-sync/cli": "latest"
  }}
}}"#,
        pkg_name, build_script, extra_scripts
    )
}

// ---------------------------------------------------------------------------
// App (SolidJS) package creation
// ---------------------------------------------------------------------------

fn write_app_package(root_path: &Path) -> Result<()> {
    let app_path = root_path.join("apps/web");
    fs::create_dir_all(app_path.join("src"))?;

    write_file(
        app_path.join("vite.config.ts"),
        r#"import { defineConfig } from 'vite';
import solidPlugin from 'vite-plugin-solid';

export default defineConfig({
  plugins: [solidPlugin()],
  server: {
    port: 3000,
  },
  build: {
    target: 'esnext',
  },
});
"#,
    )?;

    write_file(
        app_path.join("tsconfig.json"),
        r#"{
  "compilerOptions": {
    "strict": true,
    "target": "ESNext",
    "module": "ESNext",
    "moduleResolution": "node",
    "allowSyntheticDefaultImports": true,
    "esModuleInterop": true,
    "jsx": "preserve",
    "jsxImportSource": "solid-js",
    "types": ["vite/client"]
  }
}
"#,
    )?;

    write_file(
        app_path.join("package.json"),
        r#"{
  "name": "web",
  "version": "0.0.0",
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "serve": "vite preview"
  },
  "dependencies": {
    "solid-js": "^1.8.7",
    "@spooky-sync/client-solid": "latest"
  },
  "devDependencies": {
    "vite": "^5.0.0",
    "vite-plugin-solid": "^2.8.0",
    "typescript": "^5.3.0"
  }
}
"#,
    )?;

    write_file(
        app_path.join("src/index.tsx"),
        r#"/* @refresh reload */
import { render } from 'solid-js/web';
import App from './App';

const root = document.getElementById('root');

if (root instanceof HTMLElement) {
  render(() => <App />, root);
}
"#,
    )?;

    write_file(
        app_path.join("src/App.tsx"),
        r#"import type { Component } from 'solid-js';

const App: Component = () => {
  return (
    <div>
      <header>
        <h1>Welcome to Spooky</h1>
      </header>
    </div>
  );
};

export default App;
"#,
    )?;

    write_file(
        app_path.join("index.html"),
        r#"<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Spooky App</title>
  </head>
  <body>
    <div id="root"></div>
    <script src="/src/index.tsx" type="module"></script>
  </body>
</html>
"#,
    )?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn write_file<P: AsRef<Path>>(path: P, content: &str) -> Result<()> {
    if let Some(parent) = path.as_ref().parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {:?}", parent))?;
        }
    }
    fs::write(&path, content)
        .with_context(|| format!("Failed to write file {:?}", path.as_ref()))?;
    Ok(())
}
