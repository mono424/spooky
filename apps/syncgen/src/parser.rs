use anyhow::{Context, Result};
use std::collections::HashMap;
use surrealdb_core::dbs::capabilities::ExperimentalTarget;
use surrealdb_core::dbs::Capabilities;
use surrealdb_core::sql::statements::DefineStatement;
use surrealdb_core::sql::Statement;
use surrealdb_core::syn::parse_with_capabilities;

#[derive(Debug, Clone)]
pub struct TableSchema {
    #[allow(dead_code)]
    pub name: String,
    pub fields: HashMap<String, FieldDefinition>,
    pub schemafull: bool,
    pub relationships: Vec<Relationship>, // List of relationships with field info
    pub is_relation: bool,                // Whether this is a relation table
    pub relation_from: Option<String>,    // Source table for relation
    pub relation_to: Option<String>,      // Target table for relation
}

#[derive(Debug, Clone)]
pub struct Relationship {
    pub field_name: String,
    pub related_table: String,
}

#[derive(Debug, Clone)]
pub struct FieldDefinition {
    #[allow(dead_code)]
    pub name: String,
    pub field_type: FieldType,
    pub optional: bool,
    pub assert: Option<String>,
    pub value: Option<String>,
    pub is_record_id: bool,
}

#[derive(Debug, Clone)]
pub enum FieldType {
    String,
    Int,
    Float,
    Bool,
    Datetime,
    Duration,
    Array(Box<FieldType>),
    Record(String),
    Option(Box<FieldType>),
    Any,
}

pub struct SchemaParser {
    pub tables: HashMap<String, TableSchema>,
}

impl SchemaParser {
    pub fn new() -> Self {
        Self {
            tables: HashMap::new(),
        }
    }

    pub fn parse_file(&mut self, content: &str) -> Result<()> {
        // Pre-process the content to remove EVENT definitions
        // Events may contain syntax that the parser doesn't fully support yet
        let processed_content = Self::remove_events(content);

        // Create capabilities with experimental features enabled
        let capabilities = Capabilities::default()
            .with_experimental(ExperimentalTarget::RecordReferences.into())
            .with_scripting(true);

        let query = parse_with_capabilities(&processed_content, &capabilities)
            .context("Failed to parse SurrealDB schema file")?;

        self.process_statements(query.0)?;
        Ok(())
    }

    /// Remove DEFINE EVENT statements from the schema content
    /// This is a workaround for parser limitations with certain EVENT syntax
    fn remove_events(content: &str) -> String {
        let lines: Vec<&str> = content.lines().collect();
        let mut result = Vec::new();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i];

            // Check if this line starts a DEFINE EVENT
            if line.trim_start().starts_with("DEFINE EVENT") {
                // Skip lines until we find the closing semicolon or brace
                let mut brace_count = 0;
                let mut in_event = true;

                while i < lines.len() && in_event {
                    let current = lines[i];

                    // Count braces
                    for ch in current.chars() {
                        match ch {
                            '{' => brace_count += 1,
                            '}' => {
                                brace_count -= 1;
                                if brace_count == 0 {
                                    // Check if there's a semicolon on this line
                                    if current.contains(';') {
                                        in_event = false;
                                    }
                                }
                            }
                            ';' if brace_count == 0 => {
                                in_event = false;
                            }
                            _ => {}
                        }
                    }

                    i += 1;

                    // Safety check - if we've gone too far without finding the end, break
                    if i > lines.len() {
                        break;
                    }
                }

                // Continue without adding the event lines to result
                continue;
            }

            result.push(line);
            i += 1;
        }

        result.join("\n")
    }

    fn process_statements(&mut self, statements: surrealdb_core::sql::Statements) -> Result<()> {
        for statement in statements.0 {
            match statement {
                Statement::Define(define) => {
                    self.process_define_statement(define)?;
                }
                _ => {
                    // Skip other statement types (scopes, events, etc.)
                }
            }
        }
        Ok(())
    }

    fn process_define_statement(&mut self, define: DefineStatement) -> Result<()> {
        match define {
            DefineStatement::Table(table_def) => {
                let table_name = table_def.name.to_string();
                let schemafull = matches!(table_def.kind, surrealdb_core::sql::TableType::Normal);

                // Check if this is a relation table
                let is_relation =
                    matches!(table_def.kind, surrealdb_core::sql::TableType::Relation(_));
                let (relation_from, relation_to) = if is_relation {
                    if let surrealdb_core::sql::TableType::Relation(rel) = table_def.kind {
                        let from = rel.from.as_ref().and_then(|kind| {
                            // Extract table names from the Kind type
                            if let surrealdb_core::sql::Kind::Record(tables) = kind {
                                if tables.is_empty() {
                                    None
                                } else {
                                    Some(tables[0].to_string())
                                }
                            } else {
                                None
                            }
                        });
                        let to = rel.to.as_ref().and_then(|kind| {
                            // Extract table names from the Kind type
                            if let surrealdb_core::sql::Kind::Record(tables) = kind {
                                if tables.is_empty() {
                                    None
                                } else {
                                    Some(tables[0].to_string())
                                }
                            } else {
                                None
                            }
                        });
                        (from, to)
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                };

                self.tables.insert(
                    table_name.clone(),
                    TableSchema {
                        name: table_name,
                        fields: HashMap::new(),
                        schemafull,
                        relationships: Vec::new(),
                        is_relation,
                        relation_from,
                        relation_to,
                    },
                );
            }
            DefineStatement::Field(field_def) => {
                let table_name = field_def.what.to_string();
                let field_name = field_def.name.to_string();

                let field_type = if let Some(kind) = field_def.kind {
                    Self::parse_kind(kind)
                } else {
                    FieldType::Any
                };

                let assert_clause = field_def.assert.map(|v| format!("{}", v));
                let value_clause = field_def.value.map(|v| format!("{}", v));

                // Check if this field is a record ID (has Record type)
                let is_record_id = Self::is_field_record_id(&field_type);

                let field = FieldDefinition {
                    name: field_name.clone(),
                    field_type: field_type.clone(),
                    optional: false,
                    assert: assert_clause,
                    value: value_clause,
                    is_record_id,
                };

                if let Some(table) = self.tables.get_mut(&table_name) {
                    // Extract related table name from Record type
                    if let Some(related_table) = Self::extract_related_table(&field_type) {
                        let relationship = Relationship {
                            field_name: field_name.clone(),
                            related_table: related_table.clone(),
                        };
                        // Check if this exact relationship already exists
                        if !table
                            .relationships
                            .iter()
                            .any(|r| r.field_name == field_name && r.related_table == related_table)
                        {
                            table.relationships.push(relationship);
                        }
                    }
                    table.fields.insert(field_name, field);
                }
            }
            _ => {
                // Skip other define types (indexes, scopes, etc.)
            }
        }

        Ok(())
    }

    fn parse_kind(kind: surrealdb_core::sql::Kind) -> FieldType {
        use surrealdb_core::sql::Kind;

        match kind {
            Kind::String => FieldType::String,
            Kind::Int => FieldType::Int,
            Kind::Float => FieldType::Float,
            Kind::Bool => FieldType::Bool,
            Kind::Datetime => FieldType::Datetime,
            Kind::Duration => FieldType::Duration,
            Kind::Array(inner, _) => FieldType::Array(Box::new(Self::parse_kind(*inner))),
            Kind::Record(tables) => {
                if tables.is_empty() {
                    FieldType::Record("any".to_string())
                } else {
                    FieldType::Record(tables[0].to_string())
                }
            }
            Kind::Option(inner) => FieldType::Option(Box::new(Self::parse_kind(*inner))),
            Kind::Any => FieldType::Any,
            _ => FieldType::Any,
        }
    }

    /// Check if a field type is a record ID (contains Record type anywhere in the type hierarchy)
    fn is_field_record_id(field_type: &FieldType) -> bool {
        match field_type {
            FieldType::Record(_) => true,
            FieldType::Option(inner) => Self::is_field_record_id(inner),
            FieldType::Array(inner) => Self::is_field_record_id(inner),
            _ => false,
        }
    }

    /// Extract the related table name from a field type (if it's a Record type)
    fn extract_related_table(field_type: &FieldType) -> Option<String> {
        match field_type {
            FieldType::Record(table_name) if table_name != "any" => {
                // Check if this is a junction table and map it to the actual target table
                let actual_table = if table_name == "commented_on" {
                    println!("Mapping junction table {} to comment", table_name);
                    "comment".to_string()
                } else if table_name.ends_with("_on") || table_name.contains("relation") {
                    // This is likely a junction table, try to find the actual target
                    // For now, use a simple heuristic
                    println!("Junction table detected: {}", table_name);
                    table_name.clone()
                } else {
                    table_name.clone()
                };
                Some(actual_table)
            }
            FieldType::Option(inner) => Self::extract_related_table(inner),
            FieldType::Array(inner) => Self::extract_related_table(inner),
            _ => None,
        }
    }
}
