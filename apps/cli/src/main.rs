mod add_api;
mod backend;
mod bucket;
mod codegen;
mod dev;
mod json_schema;
mod migrate;
mod modules;
mod parser;
mod schema_builder;
mod schema_diff;
mod schema_extract;
mod create;
mod spooky;
mod surreal_client;

use anyhow::{Context, Result};
use backend::{BackendProcessor, SpookyConfig, DEFAULT_CONFIG_PATH};
use clap::{Args as ClapArgs, Parser as ClapParser, Subcommand};
use codegen::{CodeGenerator, OutputFormat};
use json_schema::JsonSchemaGenerator;
use parser::SchemaParser;
use create::create_project;
use std::fs;
use std::path::{Path, PathBuf};
use surreal_client::SurrealClient;

#[derive(ClapParser, Debug)]
#[command(name = "syncgen")]
#[command(about = "Generate types from SurrealDB schema files", long_about = None)]
struct Args {
    /// Path to the project directory (defaults to current directory)
    #[arg(long)]
    path: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,

    /// Path to the input .surql schema file
    #[arg(short, long)]
    input: Option<PathBuf>,

    /// Path to the output file (extension determines format: .json, .ts, .dart)
    /// Or use --format to override
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Path to the spooky.yml configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Output format (json, typescript, dart)
    /// If not specified, will be inferred from output file extension
    #[arg(short, long)]
    format: Option<String>,

    /// Pretty print the JSON output (only for JSON format)
    #[arg(short, long, default_value_t = true)]
    pretty: bool,

    /// Generate all formats (TypeScript and Dart) in addition to JSON Schema
    #[arg(short, long, default_value_t = false)]
    all: bool,

    /// Disable the generated file comment header (enabled by default)
    #[arg(long = "no-header", default_value_t = false)]
    no_header: bool,

    /// Path to another .surql file to append to the input
    #[arg(long)]
    append: Option<PathBuf>,

    /// Directory containing Surrealism modules to compile and bundle
    #[arg(long, default_value = "../../packages/surrealism-modules")]
    modules_dir: PathBuf,

    /// Generation mode: "singlenode" (HTTP to single SSP), "cluster" (HTTP to scheduler), or "surrealism" (embedded WASM)
    #[arg(long, default_value = "singlenode")]
    mode: String,

    /// SSP/Scheduler Endpoint URL (used in "singlenode" and "cluster" modes)
    #[arg(long)]
    endpoint: Option<String>,

    /// SSP/Scheduler Auth Secret (used in "singlenode" and "cluster" modes)
    #[arg(long)]
    secret: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Create a new Spooky project
    Create,
    /// Alias for 'create' (backward compat)
    #[command(hide = true)]
    Setup,
    /// Database migration management
    Migrate {
        #[command(subcommand)]
        action: MigrateCommands,
    },
    /// Bucket management
    Bucket {
        #[command(subcommand)]
        action: BucketCommands,
    },
    /// API backend management
    Api {
        #[command(subcommand)]
        action: ApiCommands,
    },
    /// Start a local development environment
    Dev {
        /// Skip migration check entirely
        #[arg(long)]
        skip_migrations: bool,
        /// Auto-apply pending migrations without prompting
        #[arg(long)]
        apply_migrations: bool,
    },
    /// Generate client types from spooky.yml
    Generate {
        /// Path to spooky.yml config file
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// Alias for 'generate'
    #[command(hide = true)]
    Gen {
        /// Path to spooky.yml config file
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum BucketCommands {
    /// Add a new storage bucket
    Add {
        /// Bucket name (snake_case, e.g. user_avatars)
        #[arg(long)]
        name: Option<String>,

        /// Preset type: avatars, images, documents, video, audio, custom
        #[arg(long)]
        preset: Option<String>,

        /// Max file size (e.g. 5mb, 500kb, 1gb)
        #[arg(long)]
        max_size: Option<String>,

        /// Allowed file extensions, comma-separated (e.g. jpg,png,gif)
        #[arg(long)]
        extensions: Option<String>,

        /// Storage backend
        #[arg(long, default_value = "memory")]
        backend: String,

        /// Enable per-user path isolation
        #[arg(long)]
        path_prefix_auth: Option<bool>,

        /// Path to spooky.yml config file
        #[arg(long)]
        config: Option<PathBuf>,

        /// Directory for bucket .surql files
        #[arg(long)]
        buckets_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum ApiCommands {
    /// Add an API backend
    Add {
        /// Path to OpenAPI spec file
        #[arg(long)]
        spec: Option<String>,

        /// Backend name (key in spooky.yml)
        #[arg(long)]
        name: Option<String>,

        /// API base URL
        #[arg(long)]
        base_url: Option<String>,

        /// Auth type (e.g. "token")
        #[arg(long)]
        auth_type: Option<String>,

        /// Auth token
        #[arg(long)]
        auth_token: Option<String>,

        /// Outbox table name
        #[arg(long)]
        table: Option<String>,

        /// Path for generated .surql schema file
        #[arg(long)]
        schema_path: Option<String>,

        /// Path to spooky.yml config file
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum MigrateCommands {
    /// Create a new migration (auto-generates diff from schema changes)
    Create {
        /// Name for the migration (e.g. "add_user_avatar")
        name: String,
        /// Path to .surql schema file to pre-populate the migration (legacy mode)
        #[arg(long)]
        schema: Option<PathBuf>,
        /// Migrations directory
        #[arg(long)]
        migrations_dir: Option<PathBuf>,
        /// Path to the input .surql schema file (for auto-diff)
        #[arg(long)]
        input: Option<PathBuf>,
        /// Path to spooky.yml config file
        #[arg(long)]
        config: Option<PathBuf>,
        /// Generation mode: singlenode, cluster, surrealism
        #[arg(long, default_value = "singlenode")]
        mode: String,
        /// SSP/Scheduler endpoint URL
        #[arg(long)]
        endpoint: Option<String>,
        /// SSP/Scheduler auth secret
        #[arg(long)]
        secret: Option<String>,
        /// SurrealDB URL for live DB schema extraction (skips ephemeral DB)
        #[arg(long)]
        url: Option<String>,
        /// SurrealDB namespace (used with --url)
        #[arg(long, default_value = "main")]
        namespace: String,
        /// SurrealDB database (used with --url)
        #[arg(long, default_value = "main")]
        database: String,
        /// SurrealDB username (used with --url)
        #[arg(long, default_value = "root")]
        username: String,
        /// SurrealDB password (used with --url)
        #[arg(long, default_value = "root")]
        password: String,
        /// Skip auto-diff and create an empty migration template
        #[arg(long)]
        empty: bool,
    },
    /// Apply all pending migrations
    Apply {
        #[command(flatten)]
        conn: ConnectionArgs,
        /// Migrations directory
        #[arg(long)]
        migrations_dir: Option<PathBuf>,
        /// Path to spooky.yml config file
        #[arg(long)]
        config: Option<PathBuf>,
        /// Generation mode: singlenode, cluster, surrealism
        #[arg(long, default_value = "singlenode")]
        mode: String,
        /// SSP/Scheduler endpoint URL
        #[arg(long)]
        endpoint: Option<String>,
        /// SSP/Scheduler auth secret
        #[arg(long)]
        secret: Option<String>,
    },
    /// Show migration status
    Status {
        #[command(flatten)]
        conn: ConnectionArgs,
        /// Migrations directory
        #[arg(long)]
        migrations_dir: Option<PathBuf>,
    },
}

#[derive(ClapArgs, Debug)]
struct ConnectionArgs {
    /// SurrealDB URL
    #[arg(long, env = "SURREAL_URL", default_value = "http://localhost:8000")]
    url: String,
    /// SurrealDB namespace
    #[arg(long, env = "SURREAL_NS", default_value = "main")]
    namespace: String,
    /// SurrealDB database
    #[arg(long, env = "SURREAL_DB", default_value = "main")]
    database: String,
    /// SurrealDB username
    #[arg(long, env = "SURREAL_USER", default_value = "root")]
    username: String,
    /// SurrealDB password
    #[arg(long, env = "SURREAL_PASS", default_value = "root")]
    password: String,
}

/// Filter schema content to remove field definitions with FOR select WHERE false
/// and make all fields (except 'id') nullable by wrapping their types in option<>
fn filter_schema_for_client(content: &str, parser: &SchemaParser) -> Result<String> {
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::new();
    let mut modified_lines: Vec<String> = Vec::new(); // Store owned strings
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Check if this line starts a DEFINE FIELD
        if trimmed.starts_with("DEFINE FIELD") {
            // Extract table and field name
            if let Some((table_name, field_name)) = extract_table_and_field_name(trimmed) {
                // Check if this field should be stripped
                if let Some(table) = parser.tables.get(&table_name) {
                    if let Some(field) = table.fields.get(&field_name) {
                        if field.should_strip {
                            // Skip this entire field definition (until semicolon)
                            println!(
                                "  → Removing field '{}' from table '{}' in client schema",
                                field_name, table_name
                            );
                            while i < lines.len() {
                                if let Some(idx) = lines[i].find(';') {
                                    // Check if there is content after the semicolon
                                    let after_semicolon = &lines[i][idx + 1..];
                                    if !after_semicolon.trim().is_empty() {
                                        result.push(after_semicolon.to_string());
                                    }
                                    i += 1;
                                    break;
                                }
                                i += 1;
                            }
                            continue;
                        }
                    }
                }
            }

            // Make all fields (except 'id') nullable by wrapping TYPE in option<>
            if let Some((_table_name, field_name)) = extract_table_and_field_name(trimmed) {
                if field_name != "id" {
                    let modified_line = make_field_nullable(line);
                    modified_lines.push(modified_line.clone());
                    result.push(modified_line);
                    i += 1;
                    continue;
                }
            }
        }

        result.push(line.to_string());
        i += 1;
    }

    Ok(result.join("\n"))
}

/// Make a DEFINE FIELD line nullable by wrapping its TYPE in option<>
/// Example: "DEFINE FIELD username ON TABLE user TYPE string"
///       -> "DEFINE FIELD username ON TABLE user TYPE option<string>"
fn make_field_nullable(line: &str) -> String {
    // Find "TYPE " in the line
    if let Some(type_pos) = line.find("TYPE ") {
        let before_type = &line[..type_pos + 5]; // Include "TYPE "
        let after_type = &line[type_pos + 5..];

        // Extract the type (everything until the next keyword or end of line)
        // Common keywords after TYPE: ASSERT, VALUE, PERMISSIONS, DEFAULT, READONLY
        let type_end = after_type
            .find(" ASSERT ")
            .or_else(|| after_type.find(" VALUE "))
            .or_else(|| after_type.find(" PERMISSIONS "))
            .or_else(|| after_type.find(" DEFAULT "))
            .or_else(|| after_type.find(" READONLY "))
            .or_else(|| after_type.find(';'))
            .unwrap_or(after_type.len());

        let type_str = after_type[..type_end].trim();
        let rest = &after_type[type_end..];

        // Check if already wrapped in option<> or if type is 'any' (can't be wrapped)
        if type_str.starts_with("option<")
            || type_str.starts_with("OPTION<")
            || type_str.eq_ignore_ascii_case("any")
        {
            // Already nullable or is 'any' type, return as-is
            line.to_string()
        } else {
            // Wrap the type in option<>
            format!("{}option<{}>{}", before_type, type_str, rest)
        }
    } else {
        // No TYPE found, return as-is
        line.to_string()
    }
}

/// Extract table and field name from a DEFINE FIELD line
/// Example: "DEFINE FIELD password ON TABLE user TYPE string"
/// Returns: Some(("user", "password"))
fn extract_table_and_field_name(line: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = line.split_whitespace().collect();

    // Look for pattern: DEFINE FIELD <name> ON TABLE <table>
    let mut field_name = None;
    let mut table_name = None;

    for i in 0..parts.len() {
        if parts[i] == "FIELD" && i + 1 < parts.len() {
            field_name = Some(parts[i + 1].to_string());
        }
        if parts[i] == "TABLE" && i + 1 < parts.len() {
            table_name = Some(parts[i + 1].to_string());
        }
    }

    if let (Some(table), Some(field)) = (table_name, field_name) {
        Some((table, field))
    } else {
        None
    }
}

fn handle_migrate(action: MigrateCommands) -> Result<()> {
    match action {
        MigrateCommands::Create {
            name,
            schema,
            migrations_dir,
            input,
            config,
            mode,
            endpoint,
            secret,
            url,
            namespace,
            database,
            username,
            password,
            empty,
        } => {
            // Load config to resolve paths
            let config_file = config.clone().unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH));
            let spooky_config = backend::load_config(&config_file);
            let resolved = spooky_config.resolved_schema();

            let resolved_input = input.unwrap_or(resolved.schema);
            let resolved_migrations = migrations_dir.unwrap_or(resolved.migrations);

            if empty {
                // Legacy: empty template or schema dump
                migrate::create(&resolved_migrations, &name, schema.as_deref(), None, None)
            } else {
                // Auto-diff mode
                let builder_config = schema_builder::SchemaBuilderConfig {
                    input_path: resolved_input,
                    config_path: Some(config_file),
                    mode,
                    endpoint,
                    secret,
                    include_functions: false,
                };

                let conn = url.as_ref().map(|u| {
                    (
                        u.as_str(),
                        namespace.as_str(),
                        database.as_str(),
                        username.as_str(),
                        password.as_str(),
                    )
                });

                migrate::create(
                    &resolved_migrations,
                    &name,
                    schema.as_deref(),
                    Some(&builder_config),
                    conn,
                )
            }
        }
        MigrateCommands::Apply {
            conn,
            migrations_dir,
            config,
            mode,
            endpoint,
            secret,
        } => {
            // Load config to resolve paths
            let config_file = config.unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH));
            let spooky_config = backend::load_config(&config_file);
            let resolved = spooky_config.resolved_schema();
            let resolved_migrations = migrations_dir.unwrap_or(resolved.migrations);

            let client = SurrealClient::new(
                &conn.url,
                &conn.namespace,
                &conn.database,
                &conn.username,
                &conn.password,
            );

            // Apply user migrations
            migrate::apply(&client, &resolved_migrations)?;

            // Apply internal Spooky schema (meta tables + events)
            let config_path_ref = if config_file.exists() {
                Some(config_file.as_path())
            } else {
                None
            };
            migrate::apply_internal_schema(
                &client,
                &resolved.schema,
                config_path_ref,
                &mode,
                endpoint.as_deref(),
                secret.as_deref(),
            )
        }
        MigrateCommands::Status {
            conn,
            migrations_dir,
        } => {
            // Load config to resolve migrations dir
            let spooky_config = backend::load_config(Path::new(DEFAULT_CONFIG_PATH));
            let resolved_migrations = migrations_dir.unwrap_or(spooky_config.resolved_schema().migrations);

            let client = SurrealClient::new(
                &conn.url,
                &conn.namespace,
                &conn.database,
                &conn.username,
                &conn.password,
            );
            migrate::status(&client, &resolved_migrations)
        }
    }
}

fn handle_api(action: ApiCommands) -> Result<()> {
    match action {
        ApiCommands::Add {
            spec,
            name,
            base_url,
            auth_type,
            auth_token,
            table,
            schema_path,
            config,
        } => {
            let resolved_config = config.unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH));
            add_api::add_api(spec, name, base_url, auth_type, auth_token, table, schema_path, resolved_config)
        }
    }
}

fn handle_bucket(action: BucketCommands) -> Result<()> {
    match action {
        BucketCommands::Add {
            name,
            preset,
            max_size,
            extensions,
            backend,
            path_prefix_auth,
            config,
            buckets_dir,
        } => {
            let resolved_config = config.unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH));
            let spooky_config = backend::load_config(&resolved_config);
            let resolved_buckets = buckets_dir.unwrap_or(spooky_config.resolved_schema().buckets_dir);

            bucket::add(
                name,
                preset,
                max_size,
                extensions,
                backend,
                path_prefix_auth,
                resolved_config,
                resolved_buckets,
            )
        }
    }
}

fn run_codegen(
    input_path: &Path,
    append_paths: &[PathBuf],
    output_path: &Path,
    output_format: OutputFormat,
    config_path: Option<&Path>,
    backend_processor: &BackendProcessor,
    no_header: bool,
    mode: &str,
    endpoint: Option<&str>,
    secret: Option<&str>,
    modules_dir: &Path,
    generate_all: bool,
) -> Result<()> {
    // Read the input file
    let mut content = fs::read_to_string(input_path)
        .context(format!("Failed to read input file: {:?}", input_path))?;

    // Append backend schemas to content
    if !backend_processor.schema_appends.is_empty() {
        content.push('\n');
        content.push_str(&backend_processor.schema_appends);
    }

    // Append embedded meta tables
    let meta_tables = include_str!("meta_tables.surql");
    let meta_tables_remote = include_str!("meta_tables_remote.surql");
    let meta_tables_client = include_str!("meta_tables_client.surql");

    content.push('\n');
    content.push_str(meta_tables);
    println!("  + Appended base meta_tables.surql");

    if matches!(output_format, OutputFormat::Surql) {
        content.push('\n');
        content.push_str(meta_tables_remote);
        println!("  + Appended meta_tables_remote.surql");
    } else {
        content.push('\n');
        content.push_str(meta_tables_client);
        println!("  + Appended meta_tables_client.surql");
    }

    // Append extra files if specified
    for append_path in append_paths {
        let append_content = fs::read_to_string(append_path)
            .context(format!("Failed to read append file: {:?}", append_path))?;
        content.push('\n');
        content.push_str(&append_content);
        println!("  + Appended schema from {:?}", append_path);
    }

    // Parse the schema
    let mut parser = SchemaParser::new();
    parser
        .parse_file(&content)
        .context("Failed to parse SurrealDB schema")?;

    // Extract buckets from separate bucket files (if any)
    if !backend_processor.bucket_schema.is_empty() {
        parser.extract_buckets(&backend_processor.bucket_schema);
    }

    // Filter the raw schema content to remove fields with FOR select WHERE false
    let mut filtered_schema_content = filter_schema_for_client(&content, &parser)?;

    // Append spooky_rv field to every table for local cache setup (client-side only)
    println!("  + Injecting spooky_rv field for local cache schema");
    for table_name in parser.tables.keys() {
        filtered_schema_content.push_str(&format!(
            "\nDEFINE FIELD spooky_rv ON TABLE {} TYPE int DEFAULT 0 PERMISSIONS FOR select, create, update WHERE true;",
            table_name
        ));
    }

    // Choose which content to use based on format
    let raw_schema_content = if matches!(output_format, OutputFormat::Surql) {
        let builder_config = schema_builder::SchemaBuilderConfig {
            input_path: input_path.to_path_buf(),
            config_path: config_path.map(|p| p.to_path_buf()),
            mode: mode.to_string(),
            endpoint: endpoint.map(|s| s.to_string()),
            secret: secret.map(|s| s.to_string()),
            include_functions: true,
        };
        let c = schema_builder::build_server_schema(&builder_config)?;
        println!("  + Built server schema via schema_builder");
        c
    } else {
        filtered_schema_content.clone()
    };

    println!(
        "Successfully parsed {} table(s) from {:?}",
        parser.tables.len(),
        input_path
    );

    for (table_name, table_schema) in &parser.tables {
        println!(
            "  - {}: {} field(s), schemafull: {}",
            table_name,
            table_schema.fields.len(),
            table_schema.schemafull
        );
    }

    // Generate JSON Schema
    let generator = JsonSchemaGenerator::new();
    let json_schema = generator.generate(&parser);

    let json_schema_string = serde_json::to_string_pretty(&json_schema)
        .context("Failed to serialize JSON Schema")?;

    fn ensure_directory_exists(path: &std::path::Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                fs::create_dir_all(parent)
                    .context(format!("Failed to create directory: {:?}", parent))?;
            }
        }
        Ok(())
    }

    if generate_all {
        println!("\nGenerating all formats...");

        let json_path = output_path.with_extension("json");
        ensure_directory_exists(&json_path)?;
        fs::write(&json_path, &json_schema_string)
            .context(format!("Failed to write JSON Schema file: {:?}", json_path))?;
        println!("  ✓ JSON Schema: {:?}", json_path);

        let ts_gen = CodeGenerator::new_with_header(OutputFormat::Typescript, !no_header);
        let ts_code = ts_gen
            .generate_with_schema(
                &json_schema_string,
                "Database",
                Some(&raw_schema_content),
                None,
                Some(&backend_processor.backend_definitions),
            )
            .context("Failed to generate TypeScript code")?;
        let ts_path = output_path.with_extension("ts");
        ensure_directory_exists(&ts_path)?;
        fs::write(&ts_path, ts_code)
            .context(format!("Failed to write TypeScript file: {:?}", ts_path))?;
        println!("  ✓ TypeScript: {:?}", ts_path);

        let dart_gen = CodeGenerator::new_with_header(OutputFormat::Dart, !no_header);
        let dart_code = dart_gen
            .generate_with_schema(
                &json_schema_string,
                "Database",
                Some(&raw_schema_content),
                None,
                Some(&backend_processor.backend_definitions),
            )
            .context("Failed to generate Dart code")?;
        let dart_path = output_path.with_extension("dart");
        ensure_directory_exists(&dart_path)?;
        fs::write(&dart_path, dart_code)
            .context(format!("Failed to write Dart file: {:?}", dart_path))?;
        println!("  ✓ Dart: {:?}", dart_path);

        println!("\nSuccessfully generated all formats!");
    } else {
        let is_client = !matches!(output_format, OutputFormat::Surql);
        let spooky_events = spooky::generate_spooky_events(
            &parser.tables,
            &content,
            is_client,
            mode,
            endpoint,
            secret,
        );

        let include_modules = mode == "surrealism";
        let generator = CodeGenerator::new(output_format, !no_header, include_modules);
        let output_content = generator
            .generate_with_schema(
                &json_schema_string,
                "Schema",
                Some(&raw_schema_content),
                Some(&spooky_events),
                Some(&backend_processor.backend_definitions),
            )
            .context("Failed to generate output code")?;

        ensure_directory_exists(output_path)?;
        fs::write(output_path, output_content)
            .context(format!("Failed to write output file: {:?}", output_path))?;

        let format_name = match output_format {
            OutputFormat::JsonSchema => "JSON Schema",
            OutputFormat::Typescript => "TypeScript",
            OutputFormat::Dart => "Dart",
            OutputFormat::Surql => "sql",
        };

        if matches!(output_format, OutputFormat::Surql) && mode == "surrealism" {
            if let Some(output_dir) = output_path.parent() {
                println!("\nProcessing Surrealism Modules...");
                if let Err(e) = modules::compile_modules(modules_dir, output_dir) {
                    eprintln!("Warning: Failed to compile modules: {}", e);
                }
            }
        }

        println!(
            "\nSuccessfully generated {} at {:?}",
            format_name, output_path
        );
    }

    Ok(())
}

fn handle_generate(config_path: &Path) -> Result<()> {
    let config_str = fs::read_to_string(config_path)
        .context(format!("Failed to read config file: {:?}", config_path))?;
    let config: SpookyConfig =
        serde_yaml::from_str(&config_str).context("Failed to parse spooky config")?;

    if config.client_types.is_empty() {
        anyhow::bail!(
            "No clientTypes entries found in {:?}. Add at least one entry to generate.",
            config_path
        );
    }

    let base_dir = config_path.parent().unwrap_or(Path::new("."));
    let resolved = config.resolved_schema();

    // Process backends once
    let mut backend_processor = BackendProcessor::new();
    backend_processor.process(config_path)?;

    for (i, ct) in config.client_types.iter().enumerate() {
        println!(
            "\n[{}/{}] Generating {} → {}",
            i + 1,
            config.client_types.len(),
            ct.format,
            ct.output
        );

        let output_format = match ct.format.to_lowercase().as_str() {
            "json" => OutputFormat::JsonSchema,
            "typescript" | "ts" => OutputFormat::Typescript,
            "dart" => OutputFormat::Dart,
            "surql" => OutputFormat::Surql,
            _ => {
                anyhow::bail!(
                    "Unknown format '{}' in clientTypes[{}]. Supported: json, typescript, dart, surql",
                    ct.format,
                    i
                );
            }
        };

        let input_path = base_dir.join(&resolved.schema);
        let append_paths: Vec<PathBuf> = Vec::new();
        let output_path = base_dir.join(&ct.output);

        run_codegen(
            &input_path,
            &append_paths,
            &output_path,
            output_format,
            Some(config_path),
            &backend_processor,
            false,
            "singlenode",
            None,
            None,
            Path::new("../../packages/surrealism-modules"),
            false,
        )?;
    }

    println!("\nAll clientTypes generated successfully.");
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();

    if let Some(ref project_path) = args.path {
        std::env::set_current_dir(project_path)
            .context(format!("Failed to set project directory: {:?}", project_path))?;
    }

    match args.command {
        Some(Commands::Create) | Some(Commands::Setup) => return create_project(),
        Some(Commands::Migrate { action }) => return handle_migrate(action),
        Some(Commands::Bucket { action }) => return handle_bucket(action),
        Some(Commands::Api { action }) => return handle_api(action),
        Some(Commands::Dev { skip_migrations, apply_migrations }) => {
            return dev::run(skip_migrations, apply_migrations);
        }
        Some(Commands::Generate { config }) | Some(Commands::Gen { config }) => {
            let resolved_config = config.unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH));
            return handle_generate(&resolved_config);
        }
        None => {} // fall through to legacy codegen mode
    }

    // Surrealism mode is not supported yet
    if args.mode == "surrealism" {
        eprintln!("Warning: Surrealism mode is not supported yet.");
        std::process::exit(1);
    }

    // Legacy mode validation
    let input_path = args
        .input
        .as_ref()
        .context("Main argument --input is required (or use 'setup' command)")?;
    let output_path = args
        .output
        .as_ref()
        .context("Main argument --output is required (or use 'setup' command)")?;

    // Determine output format
    let output_format = if let Some(format_str) = &args.format {
        match format_str.to_lowercase().as_str() {
            "json" => OutputFormat::JsonSchema,
            "typescript" | "ts" => OutputFormat::Typescript,
            "dart" => OutputFormat::Dart,
            "surql" => OutputFormat::Surql,
            _ => {
                anyhow::bail!(
                    "Unknown format: {}. Supported formats: json, typescript, dart, surql",
                    format_str
                );
            }
        }
    } else {
        // Infer from file extension
        OutputFormat::from_extension(output_path.to_str().unwrap_or(""))
            .unwrap_or(OutputFormat::JsonSchema)
    };

    // Process spooky config/backends
    let mut backend_processor = BackendProcessor::new();
    if let Some(config_path) = &args.config {
        println!("Loading spooky config from {:?}", config_path);
        backend_processor.process(config_path)?;
    }

    let append_paths: Vec<PathBuf> = args.append.iter().cloned().collect();

    run_codegen(
        input_path,
        &append_paths,
        output_path,
        output_format,
        args.config.as_deref(),
        &backend_processor,
        args.no_header,
        &args.mode,
        args.endpoint.as_deref(),
        args.secret.as_deref(),
        &args.modules_dir,
        args.all,
    )
}
