use anyhow::{Context, Result};
use std::collections::HashMap;
use surrealdb_core::sql::statements::DefineStatement;
use surrealdb_core::sql::{parse, Statement};

#[derive(Debug, Clone)]
pub struct TableSchema {
    #[allow(dead_code)]
    pub name: String,
    pub fields: HashMap<String, FieldDefinition>,
    pub schemafull: bool,
    pub relationships: Vec<String>, // List of related table names
}

#[derive(Debug, Clone)]
pub struct FieldDefinition {
    #[allow(dead_code)]
    pub name: String,
    pub field_type: FieldType,
    pub optional: bool,
    pub assert: Option<String>,
    pub value: Option<String>,
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
        let query = parse(content).context("Failed to parse SurrealDB schema file")?;

        self.process_statements(query.0)?;
        Ok(())
    }

    fn process_statements(&mut self, statements: surrealdb_core::sql::Statements) -> Result<()> {
        for statement in statements.0 {
            match statement {
                Statement::Define(define) => {
                    self.process_define_statement(define)?;
                }
                _ => {
                    // Skip other statement types (scopes, etc.)
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

                self.tables.insert(
                    table_name.clone(),
                    TableSchema {
                        name: table_name,
                        fields: HashMap::new(),
                        schemafull,
                        relationships: Vec::new(),
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

                let field = FieldDefinition {
                    name: field_name.clone(),
                    field_type: field_type.clone(),
                    optional: false,
                    assert: assert_clause,
                    value: value_clause,
                };

                if let Some(table) = self.tables.get_mut(&table_name) {
                    // Extract related table name from Record type
                    if let Some(related_table) = Self::extract_related_table(&field_type) {
                        if !table.relationships.contains(&related_table) {
                            table.relationships.push(related_table);
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

    /// Extract the related table name from a field type (if it's a Record type)
    fn extract_related_table(field_type: &FieldType) -> Option<String> {
        match field_type {
            FieldType::Record(table_name) if table_name != "any" => {
                Some(table_name.clone())
            }
            FieldType::Option(inner) => Self::extract_related_table(inner),
            FieldType::Array(inner) => Self::extract_related_table(inner),
            _ => None,
        }
    }
}
