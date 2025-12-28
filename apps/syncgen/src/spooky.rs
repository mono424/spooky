use crate::parser::{TableSchema, FieldType};
use regex::Regex;
use std::collections::HashMap;

/// Generate Spooky events for data hashing and graph synchronization
pub fn generate_spooky_events(
    tables: &HashMap<String, TableSchema>,
    raw_content: &str,
    is_client: bool,
) -> String {
    // 1. Parse @parent tags from raw content
    // pattern: DEFINE FIELD field ON TABLE table ... -- @parent
    let parent_regex = Regex::new(r"(?i)DEFINE\s+FIELD\s+(\w+)\s+ON\s+TABLE\s+(\w+).*--\s*@parent").unwrap();
    
    let mut parent_map: HashMap<String, String> = HashMap::new(); // child_table -> parent_field_name

    for cap in parent_regex.captures_iter(raw_content) {
        if let (Some(field), Some(table)) = (cap.get(1), cap.get(2)) {
            parent_map.insert(
                table.as_str().to_lowercase(),
                field.as_str().to_lowercase(),
            );
        }
    }

    // 2. Generate Events
    let mut events = String::from("\n-- ==================================================\n-- AUTO-GENERATED SPOOKY EVENTS\n-- ==================================================\n\n");

    // Client Logic: Minimal logic, only Intrinsic Hash, Dirty Flags
    if is_client {
        // Sort table names for deterministic output
        let mut sorted_table_names: Vec<_> = tables.keys().collect();
        sorted_table_names.sort();

        for table_name in &sorted_table_names {
            // Skip system/internal tables and the spooky hash tables themselves
            if table_name.starts_with("_spooky_") {
                continue;
            }

            let table = tables.get(*table_name).unwrap();
            
            if table.is_relation {
                continue;
            }

            // Calculate Intrinsic Fields
            let mut intrinsic_fields = Vec::new();
            let mut sorted_fields: Vec<_> = table.fields.keys().collect();
            sorted_fields.sort();

            for field_name in &sorted_fields {
                if **field_name == "password" || **field_name == "created_at" {
                    continue;
                }
                intrinsic_fields.push(format!("{}: $after.{}", field_name, field_name));
            }

            // --------------------------------------------------
            // A. Client Mutation Event
            // --------------------------------------------------
            events.push_str(&format!("-- Table: {} Client Mutation\n", table_name));
            events.push_str(&format!("DEFINE EVENT OVERWRITE _spooky_{}_client_mutation ON TABLE {}\n", table_name, table_name));
            events.push_str("WHEN $before != $after AND $event != \"DELETE\"\nTHEN {\n");

            // Just mark as dirty. Hashes are irrelevant/empty until synced.
            events.push_str("    UPSERT _spooky_data_hash CONTENT {\n");
            events.push_str("        RecordId: $after.id,\n");
            // Use empty strings as placeholders since type is string
            events.push_str("        IntrinsicHash: \"\",\n"); 
            events.push_str("        CompositionHash: \"\",\n");
            events.push_str("        TotalHash: NONE,\n");
            events.push_str("        IsDirty: true,\n");
            events.push_str("        PendingDelete: false\n");
            events.push_str("    };\n");
            events.push_str("};\n\n");

            // --------------------------------------------------
            // B. Client Deletion Event
            // --------------------------------------------------
            events.push_str(&format!("-- Table: {} Client Deletion\n", table_name));
            events.push_str(&format!("DEFINE EVENT OVERWRITE _spooky_{}_client_delete ON TABLE {}\n", table_name, table_name));
            events.push_str("WHEN $event = \"DELETE\"\nTHEN {\n");
            
            // Mark as PendingDelete instead of removing
            // Note: If the record is truly deleted, this event runs. But upserting purely by ID might fail if we need other data? 
            // _spooky_data_hash uses RecordId as key. So we can update it directly.
            events.push_str("    UPDATE _spooky_data_hash SET PendingDelete = true WHERE RecordId = $before.id;\n");
            events.push_str("};\n\n");
        }

        return events;
    }

    // Remote Logic: Full Merkle Tree Logic
    
    // Sort table names for deterministic output
    let mut sorted_table_names: Vec<_> = tables.keys().collect();
    sorted_table_names.sort();

    for table_name in &sorted_table_names {
        // Skip system/internal tables and the spooky hash tables themselves
        if table_name.starts_with("_spooky_") {
            continue;
        }

        let table = tables.get(*table_name).unwrap();
        
        // Skip relation tables that are explicitly marked as such (if we had that metadata easily available)
        // In the parser, we store is_relation.
        if table.is_relation {
            continue;
        }

        // --------------------------------------------------
        // A. Mutation Event
        // --------------------------------------------------
        events.push_str(&format!("-- Table: {} Mutation\n", table_name));
        events.push_str(&format!("DEFINE EVENT OVERWRITE _spooky_{}_mutation ON TABLE {}\n", table_name, table_name));
        events.push_str("WHEN $before != $after AND $event != \"DELETE\"\nTHEN {\n");
        // Inject DBSP Ingest Call
        // Args: table, operation, id, record
        // ID: Use $after.id (since it's not a DELETE)
        // Operation: $event (CREATE or UPDATE)
        // Construct Plain Record for WASM (Sanitize Record Links to Strings)
        events.push_str("    LET $plain_after = {\n");
        events.push_str("        id: <string>$after.id,\n");
        
        let mut all_fields: Vec<_> = table.fields.keys().collect();
        all_fields.sort();
        
        for field_name in all_fields {
             let field_def = table.fields.get(field_name).unwrap();
             match field_def.field_type {
                 FieldType::Record(_) | FieldType::Datetime => {
                     events.push_str(&format!("        {}: <string>$after.{},\n", field_name, field_name));
                 },
                 _ => {
                     events.push_str(&format!("        {}: $after.{},\n", field_name, field_name));
                 }
             }
        }
        events.push_str("    };\n");

        // Pass $plain_after instead of $after
        // Note: Explicitly cast to object again just in case constructed object is weird
        // Use record::id() to get clean ID string without backticks
        events.push_str(&format!("    LET $dbsp_ok = mod::dbsp::ingest('{}', $event, <string>$after.id, $plain_after);\n", table_name));
        events.push_str("    FOR $u IN $dbsp_ok.updates {\n");
        events.push_str("        UPDATE _spooky_incantation SET Hash = $u.result_hash, Tree = $u.tree WHERE Id = $u.query_id;\n");
        events.push_str("    };\n");
        events.push_str("};\n\n");

        // --------------------------------------------------
        // B. Deletion Event
        // --------------------------------------------------
        events.push_str(&format!("-- Table: {} Deletion\n", table_name));
        events.push_str(&format!("DEFINE EVENT OVERWRITE _spooky_{}_delete ON TABLE {}\n", table_name, table_name));
        events.push_str("WHEN $event = \"DELETE\"\nTHEN {\n");
        // Inject DBSP Ingest Call
        // Args: table, operation, id, record
        // ID: Use $before.id
        // Operation: "DELETE"
        // Record: $before (data being deleted)
        // Construct Plain Record for WASM
        events.push_str("    LET $plain_before = {\n");
        events.push_str("        id: <string>$before.id,\n");
        
        let mut all_fields_del: Vec<_> = table.fields.keys().collect();
        all_fields_del.sort();
        
        for field_name in all_fields_del {
             let field_def = table.fields.get(field_name).unwrap();
             match field_def.field_type {
                 FieldType::Record(_) | FieldType::Datetime => {
                     events.push_str(&format!("        {}: <string>$before.{},\n", field_name, field_name));
                 },
                 _ => {
                     events.push_str(&format!("        {}: $before.{},\n", field_name, field_name));
                 }
             }
        }
        events.push_str("    };\n");

        // Use record::id() to get clean ID string without backticks
        events.push_str(&format!("    LET $dbsp_ok = mod::dbsp::ingest('{}', \"DELETE\", <string>$before.id, $plain_before);\n", table_name));
        events.push_str("    FOR $u IN $dbsp_ok.updates {\n");
        events.push_str("        UPDATE _spooky_incantation SET Hash = $u.result_hash, Tree = $u.tree WHERE Id = $u.query_id;\n");
        events.push_str("    };\n");
        events.push_str("};\n\n");
    }

    events
}
