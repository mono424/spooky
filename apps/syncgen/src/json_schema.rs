use crate::parser::{FieldType, SchemaParser, TableSchema};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonSchema {
    #[serde(rename = "$schema")]
    pub schema: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "type")]
    pub schema_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<HashMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<Vec<String>>,
    pub definitions: HashMap<String, Value>,
}

pub struct JsonSchemaGenerator;

impl JsonSchemaGenerator {
    pub fn new() -> Self {
        Self
    }

    pub fn generate(&self, parser: &SchemaParser) -> JsonSchema {
        let mut definitions = HashMap::new();
        let mut properties = HashMap::new();
        let mut required_tables = Vec::new();

        for (table_name, table_schema) in &parser.tables {
            let definition = self.generate_table_definition(table_schema);
            definitions.insert(table_name.clone(), definition);

            // Add a property that references the definition
            properties.insert(
                table_name.clone(),
                json!({
                    "$ref": format!("#/definitions/{}", table_name)
                })
            );

            // Mark all table properties as required
            required_tables.push(table_name.clone());
        }

        JsonSchema {
            schema: "http://json-schema.org/draft-07/schema#".to_string(),
            schema_type: Some("object".to_string()),
            properties: Some(properties),
            required: Some(required_tables),
            definitions,
        }
    }

    fn generate_table_definition(&self, table: &TableSchema) -> Value {
        let mut properties = serde_json::Map::new();
        let mut required_fields = Vec::new();

        // Always add 'id' field for SurrealDB records
        properties.insert(
            "id".to_string(),
            json!({
                "type": "string",
                "description": "Record ID"
            }),
        );
        required_fields.push("id".to_string());

        for (field_name, field_def) in &table.fields {
            let field_schema = self.generate_field_schema(&field_def.field_type);

            let mut field_obj = serde_json::Map::new();

            if let Value::Object(schema_props) = field_schema {
                field_obj.extend(schema_props);
            }

            // Add description with assertions if present
            if let Some(assert) = &field_def.assert {
                field_obj.insert(
                    "description".to_string(),
                    Value::String(format!("Assert: {}", assert)),
                );
            }

            properties.insert(field_name.clone(), Value::Object(field_obj));

            // Determine if field is required
            if !field_def.optional && !matches!(field_def.field_type, FieldType::Option(_)) {
                // If there's no VALUE clause and field is not optional, it's required
                if field_def.value.is_none() {
                    required_fields.push(field_name.clone());
                }
            }
        }

        let mut definition = serde_json::Map::new();
        definition.insert("type".to_string(), Value::String("object".to_string()));
        definition.insert("properties".to_string(), Value::Object(properties));

        if !required_fields.is_empty() {
            definition.insert(
                "required".to_string(),
                Value::Array(
                    required_fields
                        .into_iter()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }

        if table.schemafull {
            definition.insert(
                "additionalProperties".to_string(),
                Value::Bool(false),
            );
        }

        Value::Object(definition)
    }

    fn generate_field_schema(&self, field_type: &FieldType) -> Value {
        match field_type {
            FieldType::String => json!({
                "type": "string"
            }),
            FieldType::Int => json!({
                "type": "integer"
            }),
            FieldType::Float => json!({
                "type": "number"
            }),
            FieldType::Bool => json!({
                "type": "boolean"
            }),
            FieldType::Datetime => json!({
                "type": "string",
                "format": "date-time"
            }),
            FieldType::Duration => json!({
                "type": "string",
                "description": "ISO 8601 duration"
            }),
            FieldType::Array(inner) => {
                let items = self.generate_field_schema(inner);
                json!({
                    "type": "array",
                    "items": items
                })
            }
            FieldType::Record(table_name) => {
                if table_name == "any" {
                    json!({
                        "type": "string",
                        "description": "Record ID"
                    })
                } else {
                    json!({
                        "type": "string",
                        "description": format!("Record ID of table: {}", table_name),
                        "pattern": format!("^{}:", table_name)
                    })
                }
            }
            FieldType::Option(inner) => {
                let mut schema = self.generate_field_schema(inner);
                if let Value::Object(ref mut obj) = schema {
                    // Make the field nullable
                    if let Some(Value::String(type_str)) = obj.get("type") {
                        obj.insert(
                            "type".to_string(),
                            json!([type_str.clone(), "null"]),
                        );
                    } else {
                        // If type is already an array or missing, add null
                        obj.insert(
                            "type".to_string(),
                            json!(["string", "null"]),
                        );
                    }
                }
                schema
            }
            FieldType::Any => json!({
                "description": "Any type"
            }),
        }
    }
}
