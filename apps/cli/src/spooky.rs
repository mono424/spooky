use crate::parser::{FieldType, TableSchema};
use regex::Regex;
use std::collections::HashMap;

/// Generate Spooky events for data hashing and graph synchronization
// ... imports ...

/// Generate Spooky events for data hashing and graph synchronization
pub fn generate_spooky_events(
    tables: &HashMap<String, TableSchema>,
    raw_content: &str,
    is_client: bool,
    mode: &str,
    sidecar_endpoint: Option<&str>,
    sidecar_secret: Option<&str>,
) -> String {
    // 2. Generate Events
    let mut events = String::from("\n-- ==================================================\n-- AUTO-GENERATED SPOOKY EVENTS\n-- ==================================================\n\n");

    // Client Logic: Minimal logic, only Intrinsic Hash, Dirty Flags
    if is_client {
        // ... (existing client logic omitted for brevity, assuming it's unchanged.
        // Logic: if I'm replacing the whole function I need to include it.
        // The instruction said "Update signature and implement sidecar logic".
        // I will replace likely the whole function or large chunks to ensure consistency.)

        // Use the existing client logic from the file view
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
            events.push_str(&format!(
                "DEFINE EVENT OVERWRITE _spooky_{}_client_mutation ON TABLE {}\n",
                table_name, table_name
            ));
            events.push_str("WHEN $before != $after AND $event != \"DELETE\"\nTHEN {\n");
            // Placeholder: Could add dirty flag logic here if needed for client-side sync tracking
            events.push_str("    -- No-op for now. Client mutation sync logic moved to DBSP.\n");
            events.push_str("};\n\n");

            // --------------------------------------------------
            // B. Client Deletion Event
            // --------------------------------------------------
            events.push_str(&format!("-- Table: {} Client Deletion\n", table_name));
            events.push_str(&format!(
                "DEFINE EVENT OVERWRITE _spooky_{}_client_delete ON TABLE {}\n",
                table_name, table_name
            ));
            events.push_str("WHEN $event = \"DELETE\"\nTHEN {\n");
            events.push_str("    -- No-op for now.\n");
            events.push_str("};\n\n");
        }

        return events;
    }

    // Remote Logic: DBSP Ingest (Surrealism) OR Sidecar HTTP Call

    let is_sidecar = mode == "sidecar";

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

        // ===================================
        // 1. MUTATION EVENT (CREATE / UPDATE)
        // ===================================
        // Merges version tracking and data ingestion
        events.push_str(&format!(
            "DEFINE EVENT OVERWRITE _spooky_{}_mutation ON TABLE {}\n",
            table_name, table_name
        ));
        events.push_str("WHEN $before != $after AND $event != \"DELETE\"\nTHEN {\n");

                // --- Versioning Logic ---
        events.push_str("    LET $spooky_ver_rec = IF $event = \"CREATE\" {\n");
        events.push_str("        (CREATE _spooky_version SET record_id = $after.id, version = 1 RETURN AFTER)\n");
        events.push_str("    } ELSE IF $event = \"UPDATE\" {\n");
        events.push_str("        IF $spooky_target_version != NONE AND $spooky_target_version.id == $after.id {\n");
        events.push_str("            LET $u = (UPDATE _spooky_version SET version = <int>$spooky_target_version.version WHERE record_id = $after.id RETURN AFTER);\n");
        events.push_str("            LET $spooky_target_version = NONE;\n");
        events.push_str("            $u\n");
        events.push_str("        } ELSE {\n");
        events.push_str("            (UPDATE _spooky_version SET version += 1 WHERE record_id = $after.id RETURN AFTER)\n");
        events.push_str("        }\n");
        events.push_str("    };\n");
        events.push_str("    LET $spooky_ver = $spooky_ver_rec[0].version;\n\n");

        // --- Ingestion Logic ---
        events.push_str("    LET $plain_after = {\n");
        events.push_str("        id: <string>($after.id OR \"\"),\n");

        let mut all_fields: Vec<_> = table.fields.keys().collect();
        all_fields.sort();

        for field_name in all_fields {
            let field_def = table.fields.get(field_name).unwrap();
            match field_def.field_type {
                FieldType::Record(_) | FieldType::Datetime => {
                    events.push_str(&format!(
                        "        {}: <string>($after.{} OR \"\"),\n",
                        field_name, field_name
                    ));
                }
                _ => {
                    events.push_str(&format!("        {}: $after.{},\n", field_name, field_name));
                }
            }
        }
        events.push_str("        _spooky_version: (SELECT VALUE version FROM ONLY _spooky_version WHERE record_id = $after.id)\n");
        events.push_str("    };\n");

        if is_sidecar {
            let endpoint = sidecar_endpoint.unwrap_or("http://localhost:8667");
            let secret = sidecar_secret.unwrap_or("");
            let url = format!("{}/ingest", endpoint);

            events.push_str("    LET $payload = {\n");
            events.push_str(&format!("        table: '{}',\n", table_name));
            events.push_str("        op: $event,\n");
            events.push_str("        id: <string>($after.id OR \"\"),\n");
            events.push_str("        record: $plain_after,\n");
            events.push_str("        hash: \"\"\n");
            events.push_str("    };\n");

            events.push_str(&format!(
                "    http::post('{}', $payload, {{ \"Authorization\": \"Bearer {}\" }});\n",
                url, secret
            ));
        } else {
            // Surrealism / WASM Mode
            events.push_str(&format!(
                "    mod::dbsp::ingest('{}', $event, <string>($after.id OR \"\"), $plain_after);\n",
                table_name
            ));
            events.push_str("    mod::dbsp::save_state(NONE);\n");
        }
        events.push_str("};\n\n");

        // ===================================
        // 2. DELETE EVENT
        // ===================================
        // Merges version cleanup and data ingestion
        events.push_str(&format!(
            "DEFINE EVENT OVERWRITE _spooky_{}_delete ON TABLE {}\n",
            table_name, table_name
        ));
        events.push_str("WHEN $event = \"DELETE\"\nTHEN {\n");

        // --- Versioning Logic ---
        events.push_str("    DELETE _spooky_version WHERE record_id = $before.id;\n\n");

        // --- Ingestion Logic ---
        events.push_str("    LET $plain_before = {\n");
        events.push_str("        id: <string>($before.id OR \"\"),\n");

        let mut all_fields_del: Vec<_> = table.fields.keys().collect();
        all_fields_del.sort();

        for field_name in all_fields_del {
            let field_def = table.fields.get(field_name).unwrap();
            match field_def.field_type {
                FieldType::Record(_) | FieldType::Datetime => {
                    events.push_str(&format!(
                        "        {}: <string>($before.{} OR \"\"),\n",
                        field_name, field_name
                    ));
                }
                _ => {
                    events.push_str(&format!(
                        "        {}: $before.{},\n",
                        field_name, field_name
                    ));
                }
            }
        }
        events.push_str("    };\n");

        if is_sidecar {
            let endpoint = sidecar_endpoint.unwrap_or("http://localhost:8667");
            let secret = sidecar_secret.unwrap_or("");
            let url = format!("{}/ingest", endpoint);

            events.push_str("    LET $payload = {\n");
            events.push_str(&format!("        table: '{}',\n", table_name));
            events.push_str("        op: \"DELETE\",\n");
            events.push_str("        id: <string>($before.id OR \"\"),\n");
            events.push_str("        record: $plain_before,\n");
            events.push_str("        hash: \"\"\n");
            events.push_str("    };\n");

            events.push_str(&format!(
                "    http::post('{}', $payload, {{ \"Authorization\": \"Bearer {}\" }});\n",
                url, secret
            ));
        } else {
            events.push_str(&format!("    mod::dbsp::ingest('{}', \"DELETE\", <string>($before.id OR \"\"), $plain_before);\n", table_name));
            events.push_str("    mod::dbsp::save_state(NONE);\n");
        }
        events.push_str("};\n\n");
    }

    events
}
