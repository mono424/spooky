use crate::parser::{TableSchema, FieldType};
use regex::Regex;
use std::collections::HashMap;

/// Generate Spooky events for data hashing and graph synchronization
// ... imports ...

/// Generate Spooky events for data hashing and graph synchronization
pub fn generate_spooky_events(
    tables: &HashMap<String, TableSchema>,
    raw_content: &str,
    is_client: bool,
) -> String {
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

            // --------------------------------------------------
            // A. Client Mutation Event
            // --------------------------------------------------
            events.push_str(&format!("-- Table: {} Client Mutation\n", table_name));
            events.push_str(&format!("DEFINE EVENT OVERWRITE _spooky_{}_client_mutation ON TABLE {}\n", table_name, table_name));
            events.push_str("WHEN $before != $after AND $event != \"DELETE\"\nTHEN {\n");
            // Placeholder: Could add dirty flag logic here if needed for client-side sync tracking
            events.push_str("    -- No-op for now. Client mutation sync logic moved to DBSP.\n");
            events.push_str("};\n\n");

            // --------------------------------------------------
            // B. Client Deletion Event
            // --------------------------------------------------
            events.push_str(&format!("-- Table: {} Client Deletion\n", table_name));
            events.push_str(&format!("DEFINE EVENT OVERWRITE _spooky_{}_client_delete ON TABLE {}\n", table_name, table_name));
            events.push_str("WHEN $event = \"DELETE\"\nTHEN {\n");
             events.push_str("    -- No-op for now.\n");
            events.push_str("};\n\n");
        }

        return events;
    }

    // Remote Logic: DBSP Ingest Only
    
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
        // Construct Plain Record for WASM (Sanitize Record Links to Strings)
        events.push_str("    LET $plain_after = {\n");
        events.push_str("        id: <string>($after.id OR \"\"),\n");
        
        let mut all_fields: Vec<_> = table.fields.keys().collect();
        all_fields.sort();
        
        for field_name in all_fields {
             let field_def = table.fields.get(field_name).unwrap();
             match field_def.field_type {
                 FieldType::Record(_) | FieldType::Datetime => {
                     events.push_str(&format!("        {}: <string>($after.{} OR \"\"),\n", field_name, field_name));
                 },
                 _ => {
                     events.push_str(&format!("        {}: $after.{},\n", field_name, field_name));
                 }
             }
        }
        events.push_str("    };\n");

        events.push_str(&format!("    LET $dbsp_ok = mod::dbsp::ingest('{}', $event, <string>($after.id OR \"\"), $plain_after);\n", table_name));
        // Handle Updates
        events.push_str("    FOR $u IN $dbsp_ok.updates {\n");
        events.push_str("        UPDATE _spooky_incantation SET hash = $u.result_hash, tree = $u.tree WHERE id = $u.query_id;\n");
        events.push_str("    };\n");
        events.push_str("    LET $saved = mod::dbsp::save_state(NONE);\n");
        events.push_str("};\n\n");

        // --------------------------------------------------
        // B. Deletion Event
        // --------------------------------------------------
        events.push_str(&format!("-- Table: {} Deletion\n", table_name));
        events.push_str(&format!("DEFINE EVENT OVERWRITE _spooky_{}_delete ON TABLE {}\n", table_name, table_name));
        events.push_str("WHEN $event = \"DELETE\"\nTHEN {\n");
        
        // Construct Plain Record for WASM
        events.push_str("    LET $plain_before = {\n");
        events.push_str("        id: <string>($before.id OR \"\"),\n");
        
        let mut all_fields_del: Vec<_> = table.fields.keys().collect();
        all_fields_del.sort();
        
        for field_name in all_fields_del {
             let field_def = table.fields.get(field_name).unwrap();
             match field_def.field_type {
                 FieldType::Record(_) | FieldType::Datetime => {
                     events.push_str(&format!("        {}: <string>($before.{} OR \"\"),\n", field_name, field_name));
                 },
                 _ => {
                     events.push_str(&format!("        {}: $before.{},\n", field_name, field_name));
                 }
             }
        }
        events.push_str("    };\n");

        events.push_str(&format!("    LET $dbsp_ok = mod::dbsp::ingest('{}', \"DELETE\", <string>($before.id OR \"\"), $plain_before);\n", table_name));
        events.push_str("    FOR $u IN $dbsp_ok.updates {\n");
        events.push_str("        UPDATE _spooky_incantation SET hash = $u.result_hash, tree = $u.tree WHERE id = $u.query_id;\n");
        events.push_str("    };\n");
        events.push_str("    LET $saved = mod::dbsp::save_state(NONE);\n");
        events.push_str("};\n\n");
    }

    events
}
