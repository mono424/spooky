use crate::parser::TableSchema;
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

            for field_name in sorted_fields {
                if field_name == "password" || field_name == "created_at" {
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

            // 1. New Intrinsic Hash
            events.push_str("    LET $new_intrinsic = crypto::blake3(<string>{\n");
            for (i, field_expr) in intrinsic_fields.iter().enumerate() {
                let comma = if i < intrinsic_fields.len() - 1 { "," } else { "" };
                events.push_str(&format!("        {}{}\n", field_expr, comma));
            }
            events.push_str("    });\n\n");

            // 3. Upsert Meta Table with IsDirty = true
            events.push_str("    UPSERT _spooky_data_hash CONTENT {\n");
            events.push_str("        RecordId: $after.id,\n");
            events.push_str("        IntrinsicHash: $new_intrinsic,\n");
            events.push_str("        CompositionHash: crypto::blake3(\"\"), -- Empty for client\n");
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
    
    // Generate the _spooky_data_hash mutation event first
    events.push_str("-- Meta Table: _spooky_data_hash Mutation\n");
    // Table definition is now in meta_tables.surql

    events.push_str("-- Automatically recalculates TotalHash when IntrinsicHash or CompositionHash changes\n");
    events.push_str("DEFINE EVENT OVERWRITE _spooky_data_hash_mutation ON TABLE _spooky_data_hash\n");
    events.push_str("WHEN $before != $after AND $event != \"DELETE\"\n");
    events.push_str("THEN {\n");
    events.push_str("    LET $new_total = mod::xor::blake3_xor($after.IntrinsicHash, $after.CompositionHash);\n");
    events.push_str("    IF $new_total != $after.TotalHash THEN {\n");
    events.push_str("        UPDATE _spooky_data_hash SET TotalHash = $new_total WHERE RecordId = $after.RecordId;\n");
    events.push_str("    } END;\n");
    events.push_str("};\n\n");

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

        // Get parent field for this table
        let parent_field_opt = parent_map.get(&table_name.to_lowercase());

        // Calculate Intrinsic Fields
        // We include all fields that are not virtual/reverse relationships
        // IN the rust parser, we don't strictly have "virtual" fields in the fields map unless they were parsed from DEFINE FIELD
        // But we should exclude the parent field if it's purely a reference? 
        // Logic from TS: exclude "x-is-reverse-relationship". 
        // In Rust parser, we have `TableSchema` which strictly contains parsed `DEFINE FIELD`s. 
        // So we should include all fields present in the schema map.
        
        let mut intrinsic_fields = Vec::new();
        // Sort fields for deterministic output
        let mut sorted_fields: Vec<_> = table.fields.keys().collect();
        sorted_fields.sort();

        for field_name in sorted_fields {
            if field_name == "password" || field_name == "created_at" {
                continue;
            }
            // Logic from TS: `intrinsicFields.push("${propName}: $after.${propName}")`
            // We just format it similarly
            intrinsic_fields.push(format!("{}: $after.{}", field_name, field_name));
        }

        // --------------------------------------------------
        // A. Mutation Event
        // --------------------------------------------------
        events.push_str(&format!("-- Table: {} Mutation\n", table_name));
        events.push_str(&format!("DEFINE EVENT OVERWRITE _spooky_{}_mutation ON TABLE {}\n", table_name, table_name));
        events.push_str("WHEN $before != $after AND $event != \"DELETE\"\nTHEN {\n");

        // 1. New Intrinsic Hash
        // NOTE: crypto::blake3 expects a string and returns hex string
        events.push_str("    LET $new_intrinsic = crypto::blake3(<string>{\n");
        for (i, field_expr) in intrinsic_fields.iter().enumerate() {
            let comma = if i < intrinsic_fields.len() - 1 { "," } else { "" };
            events.push_str(&format!("        {}{}\n", field_expr, comma));
        }
        events.push_str("    });\n\n");

        // 2. Previous Hash State (use $before.id for UPDATE, fallback to $after.id for CREATE)
        events.push_str("    LET $record_id = $before.id OR $after.id;\n");
        events.push_str("    LET $old_hash_data = (SELECT * FROM ONLY _spooky_data_hash WHERE RecordId = $record_id);\n");
        events.push_str("    LET $old_total = IF $old_hash_data.RecordId != NONE THEN $old_hash_data.TotalHash ELSE $new_intrinsic END;\n");
        events.push_str("    LET $composition = IF $old_hash_data.RecordId != NONE THEN $old_hash_data.CompositionHash ELSE crypto::blake3(\"\") END;\n\n");

        // 3. Upsert Meta Table (TotalHash will be calculated by _spooky_data_hash event)
        events.push_str("    UPSERT _spooky_data_hash CONTENT {\n");
        events.push_str("        RecordId: $after.id,\n");
        events.push_str("        IntrinsicHash: $new_intrinsic,\n");
        events.push_str("        CompositionHash: $composition,\n");
        events.push_str("        TotalHash: NONE -- Placeholder, will be recalculated by event\n");
        events.push_str("    };\n\n");

        // 4. Bubble Up (If Parent exists)
        if let Some(parent_field) = parent_field_opt {
             events.push_str(&format!("    -- BUBBLE UP to Parent ({})\n", parent_field));
             events.push_str("    LET $old_total = IF $old_hash_data.RecordId != NONE THEN $old_hash_data.TotalHash ELSE $new_intrinsic END;\n");
             events.push_str("    LET $new_total_after = (SELECT TotalHash FROM ONLY _spooky_data_hash WHERE RecordId = $after.id).TotalHash;\n");
             events.push_str(&format!("    IF $before.{} = $after.{} THEN {{\n", parent_field, parent_field));
             events.push_str("        LET $delta = mod::xor::blake3_xor($old_total, $new_total_after);\n");
             events.push_str("        UPDATE _spooky_data_hash SET\n");
             events.push_str("            CompositionHash = mod::xor::blake3_xor(CompositionHash, $delta)\n");
             events.push_str(&format!("        WHERE RecordId = $after.{};\n", parent_field));
             events.push_str("    } ELSE {\n");

             // Remove contribution from Old Parent
             events.push_str("        UPDATE _spooky_data_hash SET\n");
             events.push_str("            CompositionHash = mod::xor::blake3_xor(CompositionHash, $old_total)\n");
             events.push_str(&format!("        WHERE RecordId = $before.{} AND RecordId != NONE;\n\n", parent_field));

             // Add contribution to New Parent
             events.push_str("        UPDATE _spooky_data_hash SET\n");
             events.push_str("            CompositionHash = mod::xor::blake3_xor(CompositionHash, $new_total_after)\n");
             events.push_str(&format!("        WHERE RecordId = $after.{} AND RecordId != NONE;\n", parent_field));
             events.push_str("    } END;\n\n");
        }

        // 6. Cascade Down (Incoming References) + Manual Bubble Up
        let mut cascade_updates = String::new();
        let mut bubble_up_updates = String::new();
        let mut affected_dependent_tables = Vec::new();

        for other_table_name in sorted_table_names.iter() {
             if *other_table_name == *table_name { continue; }

             let other_table = tables.get(*other_table_name).unwrap();

             for rel in &other_table.relationships {
                 if rel.related_table == **table_name {
                     let other_parent_field = parent_map.get(&other_table_name.to_lowercase());
                     let is_parent_link = if let Some(p_field) = other_parent_field {
                         p_field == &rel.field_name
                     } else {
                         false
                     };

                     if !is_parent_link {
                         cascade_updates.push_str(&format!("        -- Update {} records that reference {}\n", other_table_name, table_name));
                         cascade_updates.push_str("        UPDATE _spooky_data_hash SET\n");
                         cascade_updates.push_str("            IntrinsicHash = mod::xor::blake3_xor(IntrinsicHash, $intrinsic_delta)\n");
                         cascade_updates.push_str(&format!("        WHERE RecordId IN (SELECT value id FROM {} WHERE {} = $after.id);\n\n", other_table_name, rel.field_name));
                         affected_dependent_tables.push((other_table_name.as_str(), rel.field_name.as_str()));
                     }
                 }
             }
        }

        if !affected_dependent_tables.is_empty() {
            bubble_up_updates.push_str("\n        -- ==================================================\n");
            bubble_up_updates.push_str("        -- MANUAL BUBBLE UP\n");
            bubble_up_updates.push_str("        -- Because we updated _spooky_data_hash directly,\n");
            bubble_up_updates.push_str("        -- the dependent table's event won't fire.\n");
            bubble_up_updates.push_str("        -- We must manually propagate changes to parent records.\n");
            bubble_up_updates.push_str("        -- ==================================================\n\n");

            for (dependent_table, reference_field) in &affected_dependent_tables {
                if let Some(parent_field) = parent_map.get(&dependent_table.to_lowercase()) {
                    bubble_up_updates.push_str(&format!("        -- Bubble up from {} to its parent via {}\n", dependent_table, parent_field));
                    bubble_up_updates.push_str(&format!("        LET $affected_parents_{} = (\n", dependent_table));
                    bubble_up_updates.push_str(&format!("            SELECT count() AS count, {} \n", parent_field));
                    bubble_up_updates.push_str(&format!("            FROM {} \n", dependent_table));
                    bubble_up_updates.push_str(&format!("            WHERE {} = $after.id \n", reference_field));
                    bubble_up_updates.push_str(&format!("            GROUP BY {}\n", parent_field));
                    bubble_up_updates.push_str("        );\n\n");

                    bubble_up_updates.push_str(&format!("        FOR $item IN $affected_parents_{} {{\n", dependent_table));
                    bubble_up_updates.push_str("            IF $item.count % 2 == 1 {\n");
                    bubble_up_updates.push_str("                UPDATE _spooky_data_hash SET\n");
                    bubble_up_updates.push_str("                    CompositionHash = mod::xor::blake3_xor(CompositionHash, $intrinsic_delta)\n");
                    bubble_up_updates.push_str(&format!("                WHERE RecordId = $item.{};\n", parent_field));
                    bubble_up_updates.push_str("            }\n");
                    bubble_up_updates.push_str("        };\n\n");
                }
            }
        }

        if !cascade_updates.is_empty() {
             events.push_str("    -- ==================================================\n");
             events.push_str("    -- CASCADE DOWN (Strict Compliance)\n");
             events.push_str(&format!("    -- Logic: {} Change -> Updates Dependent INTRINSIC Hash\n", table_name));
             events.push_str("    -- ==================================================\n");
             events.push_str("    IF $old_hash_data.RecordId != NONE AND $new_intrinsic != $old_hash_data.IntrinsicHash THEN {\n");
             events.push_str("        LET $intrinsic_delta = mod::xor::blake3_xor($new_intrinsic, $old_hash_data.IntrinsicHash);\n\n");
             events.push_str(&cascade_updates);
             events.push_str(&bubble_up_updates);
             events.push_str("    } END;\n\n");
        }

        events.push_str("};\n\n");

        // --------------------------------------------------
        // B. Deletion Event
        // --------------------------------------------------
        events.push_str(&format!("-- Table: {} Deletion\n", table_name));
        events.push_str(&format!("DEFINE EVENT OVERWRITE _spooky_{}_delete ON TABLE {}\n", table_name, table_name));
        events.push_str("WHEN $event = \"DELETE\"\nTHEN {\n");
        events.push_str("    LET $old_hash_data = (SELECT * FROM ONLY _spooky_data_hash WHERE RecordId = $before.id);\n");
        events.push_str("    LET $old_total = $old_hash_data.TotalHash;\n\n");

        if let Some(parent_field) = parent_field_opt {
             events.push_str(&format!("    -- BUBBLE UP Delete to Parent\n"));
             events.push_str(&format!("    IF $old_total != NONE AND $before.{} != NONE THEN {{\n", parent_field));
             events.push_str("        UPDATE _spooky_data_hash SET\n");
             events.push_str("            CompositionHash = mod::xor::blake3_xor(CompositionHash, $old_total)\n");
             events.push_str(&format!("        WHERE RecordId = $before.{};\n", parent_field));
             events.push_str("    } END;\n\n");
        }

        events.push_str("    DELETE _spooky_data_hash WHERE RecordId = $before.id;\n");
        events.push_str("};\n\n");
    }

    events
}
