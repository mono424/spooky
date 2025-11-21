mod codegen;
mod json_schema;
mod parser;

use anyhow::{Context, Result};
use clap::Parser as ClapParser;
use codegen::{CodeGenerator, OutputFormat};
use json_schema::JsonSchemaGenerator;
use parser::SchemaParser;
use std::fs;
use std::path::PathBuf;

#[derive(ClapParser, Debug)]
#[command(name = "syncgen")]
#[command(about = "Generate types from SurrealDB schema files", long_about = None)]
struct Args {
    /// Path to the input .surql schema file
    #[arg(short, long)]
    input: PathBuf,

    /// Path to the output file (extension determines format: .json, .ts, .dart)
    /// Or use --format to override
    #[arg(short, long)]
    output: PathBuf,

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
}

/// Filter schema content to remove field definitions with FOR select WHERE false
fn filter_schema_for_client(content: &str, parser: &SchemaParser) -> Result<String> {
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::new();
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
                            println!("  → Removing field '{}' from table '{}' in client schema", field_name, table_name);
                            while i < lines.len() {
                                if lines[i].contains(';') {
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
        }

        result.push(line);
        i += 1;
    }

    Ok(result.join("\n"))
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

    // Read the input file
    let content = fs::read_to_string(&args.input)
        .context(format!("Failed to read input file: {:?}", args.input))?;

    // Parse the schema
    let mut parser = SchemaParser::new();
    parser
        .parse_file(&content)
        .context("Failed to parse SurrealDB schema")?;

    // Filter the raw schema content to remove fields with FOR select WHERE false
    let raw_schema_content = filter_schema_for_client(&content, &parser)?;

    println!(
        "Successfully parsed {} table(s) from {:?}",
        parser.tables.len(),
        args.input
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

    // Determine output format
    let output_format = if let Some(format_str) = &args.format {
        match format_str.to_lowercase().as_str() {
            "json" => OutputFormat::JsonSchema,
            "typescript" | "ts" => OutputFormat::Typescript,
            "dart" => OutputFormat::Dart,
            _ => {
                anyhow::bail!(
                    "Unknown format: {}. Supported formats: json, typescript, dart",
                    format_str
                );
            }
        }
    } else {
        // Infer from file extension
        OutputFormat::from_extension(args.output.to_str().unwrap_or(""))
            .unwrap_or(OutputFormat::JsonSchema)
    };

    if args.all {
        // Generate all formats
        println!("\nGenerating all formats...");

        // Write JSON Schema
        let json_path = args.output.with_extension("json");
        fs::write(&json_path, &json_schema_string)
            .context(format!("Failed to write JSON Schema file: {:?}", json_path))?;
        println!("  ✓ JSON Schema: {:?}", json_path);

        // Generate TypeScript
        let ts_gen = CodeGenerator::new_with_header(OutputFormat::Typescript, !args.no_header);
        let ts_code = ts_gen
            .generate_with_schema(&json_schema_string, "Database", Some(&raw_schema_content))
            .context("Failed to generate TypeScript code")?;
        let ts_path = args.output.with_extension("ts");
        fs::write(&ts_path, ts_code)
            .context(format!("Failed to write TypeScript file: {:?}", ts_path))?;
        println!("  ✓ TypeScript: {:?}", ts_path);

        // Generate Dart
        let dart_gen = CodeGenerator::new_with_header(OutputFormat::Dart, !args.no_header);
        let dart_code = dart_gen
            .generate_with_schema(&json_schema_string, "Database", Some(&raw_schema_content))
            .context("Failed to generate Dart code")?;
        let dart_path = args.output.with_extension("dart");
        fs::write(&dart_path, dart_code)
            .context(format!("Failed to write Dart file: {:?}", dart_path))?;
        println!("  ✓ Dart: {:?}", dart_path);

        println!("\nSuccessfully generated all formats!");
    } else {
        // Generate single format
        let code_gen = CodeGenerator::new_with_header(output_format, !args.no_header);
        let output_content = code_gen
            .generate_with_schema(&json_schema_string, "Database", Some(&raw_schema_content))
            .context("Failed to generate code")?;

        fs::write(&args.output, output_content)
            .context(format!("Failed to write output file: {:?}", args.output))?;

        let format_name = match output_format {
            OutputFormat::JsonSchema => "JSON Schema",
            OutputFormat::Typescript => "TypeScript",
            OutputFormat::Dart => "Dart",
        };

        println!(
            "\nSuccessfully generated {} at {:?}",
            format_name, args.output
        );
    }

    Ok(())
}
