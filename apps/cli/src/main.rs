mod codegen;
mod json_schema;
mod modules;
mod parser;
mod setup;
mod spooky;

use anyhow::{Context, Result};
use clap::{Parser as ClapParser, Subcommand};
use codegen::{CodeGenerator, OutputFormat};
use json_schema::JsonSchemaGenerator;
use parser::SchemaParser;
use setup::setup_project;
use std::fs;
use std::path::PathBuf;

#[derive(ClapParser, Debug)]
#[command(name = "syncgen")]
#[command(about = "Generate types from SurrealDB schema files", long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Path to the input .surql schema file
    #[arg(short, long)]
    input: Option<PathBuf>,

    /// Path to the output file (extension determines format: .json, .ts, .dart)
    /// Or use --format to override
    #[arg(short, long)]
    output: Option<PathBuf>,

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

    /// Generation mode: "surrealism" (embedded WASM) or "sidecar" (HTTP calls)
    #[arg(long, default_value = "surrealism")]
    mode: String,

    /// Spooky Sidecar Endpoint URL (required if mode is "sidecar")
    #[arg(long)]
    sidecar_endpoint: Option<String>,

    /// Spooky Sidecar Auth Secret (required if mode is "sidecar")
    #[arg(long)]
    sidecar_secret: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Setup a new Spooky project
    Setup,
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
            || type_str.eq_ignore_ascii_case("any") {
            // Already nullable or is 'any' type, return as-is
            line.to_string()
        } else {
            // Wrap the type in option<>
            format!("{}option<{}>{}",  before_type, type_str, rest)
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

fn main() -> Result<()> {
    let args = Args::parse();

    if let Some(Commands::Setup) = args.command {
        return setup_project();
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

    // Read the input file
    let mut content = fs::read_to_string(input_path)
        .context(format!("Failed to read input file: {:?}", input_path))?;

    // Append embedded meta tables
    let meta_tables = include_str!("meta_tables.surql");
    let meta_tables_remote = include_str!("meta_tables_remote.surql");
    let functions_remote = include_str!("functions_remote.surql");
    let functions_remote_sidecar = include_str!("functions_remote_sidecar.surql");
    let functions_remote_surrealism = include_str!("functions_remote_surrealism.surql");
    let meta_tables_client = include_str!("meta_tables_client.surql");

    // Include base meta tables
    content.push('\n');
    content.push_str(meta_tables);
    println!("  + Appended base meta_tables.surql");

    // Include format-specific meta tables
    if matches!(output_format, OutputFormat::Surql) {
        content.push('\n');
        content.push_str(meta_tables_remote);
        println!("  + Appended meta_tables_remote.surql");
    } else {
        content.push('\n');
        content.push_str(meta_tables_client);
        println!("  + Appended meta_tables_client.surql");
    }

    // Append extra file if specified
    if let Some(append_path) = &args.append {
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

    // Filter the raw schema content to remove fields with FOR select WHERE false
    let filtered_schema_content = filter_schema_for_client(&content, &parser)?;

    // Choose which content to use based on format
    let raw_schema_content = if matches!(output_format, OutputFormat::Surql) {
        let mut c = content.clone();
        c.push('\n');
        c.push_str(functions_remote);
        println!("  + Appended functions_remote.surql (common)");

        if args.mode == "sidecar" {
            println!("  → Sidecar mode detected: Using sidecar specific remote functions");
            let endpoint = args
                .sidecar_endpoint
                .as_deref()
                .unwrap_or("http://localhost:8667");
            let secret = args.sidecar_secret.as_deref().unwrap_or("");

            // Inject variables into sidecar template
            let mut sidecar_fn = functions_remote_sidecar.to_string();
            sidecar_fn = sidecar_fn.replace("{{SIDECAR_ENDPOINT}}", endpoint);
            sidecar_fn = sidecar_fn.replace("{{SIDECAR_SECRET}}", secret);

            c.push('\n');
            c.push_str(&sidecar_fn);
            println!("  + Appended functions_remote_sidecar.surql (injected)");

            // Replace unregister_view (still needed as it's in meta_tables_remote.surql)
            // We need to match the exact string from meta_tables_remote.surql
            let unregister_call = "let $result = mod::dbsp::unregister_view(<string>$before.id);";
            let unregister_http = format!(
                "let $payload = {{ id: <string>$before.id }};\n    let $result = http::post('{}/view/unregister', $payload, {{ \"Authorization\": \"Bearer {}\" }});",
                endpoint, secret
            );
            c = c.replace(unregister_call, &unregister_http);
        } else {
            c.push('\n');
            c.push_str(functions_remote_surrealism);
            println!("  + Appended functions_remote_surrealism.surql");
        }
        c
    } else {
        filtered_schema_content.clone()
    };

    println!(
        "Successfully parsed {} table(s) from {:?}",
        parser.tables.len(),
        input_path
    );

    // List parsed tables
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

    // Serialize to JSON
    let json_schema_string = if args.pretty {
        serde_json::to_string_pretty(&json_schema).context("Failed to serialize JSON Schema")?
    } else {
        serde_json::to_string(&json_schema).context("Failed to serialize JSON Schema")?
    };

    fn ensure_directory_exists(path: &std::path::Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                fs::create_dir_all(parent)
                    .context(format!("Failed to create directory: {:?}", parent))?;
            }
        }
        Ok(())
    }

    if args.all {
        // Generate all formats
        println!("\nGenerating all formats...");

        // Write JSON Schema
        let json_path = output_path.with_extension("json");
        ensure_directory_exists(&json_path)?;
        fs::write(&json_path, &json_schema_string)
            .context(format!("Failed to write JSON Schema file: {:?}", json_path))?;
        println!("  ✓ JSON Schema: {:?}", json_path);

        // Generate TypeScript
        let ts_gen = CodeGenerator::new_with_header(OutputFormat::Typescript, !args.no_header);
        let ts_code = ts_gen
            .generate_with_schema(
                &json_schema_string,
                "Database",
                Some(&raw_schema_content),
                None,
            )
            .context("Failed to generate TypeScript code")?;
        let ts_path = output_path.with_extension("ts");
        ensure_directory_exists(&ts_path)?;
        fs::write(&ts_path, ts_code)
            .context(format!("Failed to write TypeScript file: {:?}", ts_path))?;
        println!("  ✓ TypeScript: {:?}", ts_path);

        // Generate Dart
        let dart_gen = CodeGenerator::new_with_header(OutputFormat::Dart, !args.no_header);
        let dart_code = dart_gen
            .generate_with_schema(
                &json_schema_string,
                "Database",
                Some(&raw_schema_content),
                None,
            )
            .context("Failed to generate Dart code")?;
        let dart_path = output_path.with_extension("dart");
        ensure_directory_exists(&dart_path)?;
        fs::write(&dart_path, dart_code)
            .context(format!("Failed to write Dart file: {:?}", dart_path))?;
        println!("  ✓ Dart: {:?}", dart_path);

        println!("\nSuccessfully generated all formats!");
    } else {
        // Generate single format
        // Generate spooky events
        let is_client = !matches!(output_format, OutputFormat::Surql);
        let spooky_events = spooky::generate_spooky_events(
            &parser.tables,
            &content,
            is_client,
            &args.mode,
            args.sidecar_endpoint.as_deref(),
            args.sidecar_secret.as_deref(),
        );

        // Generate code
        let include_modules = args.mode == "surrealism";
        let generator = CodeGenerator::new(output_format, !args.no_header, include_modules);
        let output_content = generator
            .generate_with_schema(
                &json_schema_string,
                "Schema",
                Some(&raw_schema_content),
                Some(&spooky_events),
            )
            .context("Failed to generate output code")?;

        ensure_directory_exists(output_path)?;
        fs::write(output_path, output_content)
            .context(format!("Failed to write output file: {:?}", output_path))?;

        let format_name = match output_format {
            OutputFormat::JsonSchema => "JSON Schema",
            OutputFormat::Typescript => "TypeScript",
            OutputFormat::Dart => "Dart",
            OutputFormat::Surql => "SurrealQL",
        };

        if matches!(output_format, OutputFormat::Surql) && args.mode == "surrealism" {
            // Compile and bundle modules
            // Output dir is the directory containing args.output
            if let Some(output_dir) = output_path.parent() {
                println!("\nProcessing Surrealism Modules...");
                if let Err(e) = modules::compile_modules(&args.modules_dir, output_dir) {
                    eprintln!("Warning: Failed to compile modules: {}", e);
                    // Don't fail the whole build for this? Or should we?
                    // User said "compile and add", implies part of the process.
                    // But if directory doesn't exist, we skip.
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
