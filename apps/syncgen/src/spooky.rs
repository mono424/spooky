use crate::parser::TableSchema;
use regex::Regex;
use std::collections::HashMap;

/// Generate Spooky events for data hashing and graph synchronization
pub fn generate_spooky_events(
    tables: &HashMap<String, TableSchema>,
    raw_content: &str,
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

    // Sort table names for deterministic output
    let mut sorted_table_names: Vec<_> = tables.keys().collect();
    sorted_table_names.sort();

    for table_name in &sorted_table_names {
        // Skip system/internal tables and the spooky hash tables themselves
        if table_name.starts_with("_spooky_") || table_name.as_str() == "user" {
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
            if field_name == "password" {
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
        events.push_str("WHEN $before != $after\nTHEN {\n");

        // 1. New Intrinsic Hash
        // NOTE: crypto::blake3 takes a value. In TS it passed an object `{ field: value }`.
        // SurrealQL `crypto::blake3({ ... })` works.
        events.push_str("    LET $new_intrinsic = crypto::blake3({\n");
        for (i, field_expr) in intrinsic_fields.iter().enumerate() {
            let comma = if i < intrinsic_fields.len() - 1 { "," } else { "" };
            events.push_str(&format!("        {}{}\n", field_expr, comma));
        }
        events.push_str("    });\n\n");

        // 2. Previous Hash State
        events.push_str("    LET $old_hash_data = (SELECT * FROM ONLY _spooky_data_hash WHERE RecordId = $before.id);\n");
        events.push_str("    LET $old_total = $old_hash_data.TotalHash OR <bytes>[];\n");
        events.push_str("    LET $composition = $old_hash_data.CompositionHash OR <bytes>[];\n\n");

        // 3. New Total Hash
        events.push_str("    LET $new_total = array::boolean_xor($new_intrinsic, $composition);\n\n");

        // 4. Upsert Meta Table
        events.push_str("    UPSERT _spooky_data_hash CONTENT {\n");
        events.push_str("        RecordId: $after.id,\n");
        events.push_str("        IntrinsicHash: $new_intrinsic,\n");
        events.push_str("        CompositionHash: $composition,\n");
        events.push_str("        TotalHash: $new_total\n");
        events.push_str("    };\n\n");

        // 5. Bubble Up (If Parent exists)
        if let Some(parent_field) = parent_field_opt {
             events.push_str(&format!("    -- BUBBLE UP to Parent ({})\n", parent_field));
             events.push_str(&format!("    IF $before.{} = $after.{} THEN {{\n", parent_field, parent_field));
             events.push_str("        LET $delta = array::boolean_xor($old_total, $new_total);\n");
             events.push_str("        UPDATE _spooky_data_hash SET\n");
             events.push_str("            CompositionHash = array::boolean_xor(CompositionHash, $delta),\n");
             events.push_str("            TotalHash = array::boolean_xor(IntrinsicHash, array::boolean_xor(CompositionHash, $delta))\n");
             events.push_str(&format!("        WHERE RecordId = $after.{};\n", parent_field));
             events.push_str("    } ELSE {\n");
             
             // Remove contribution from Old Parent
             events.push_str("        UPDATE _spooky_data_hash SET\n");
             events.push_str("            CompositionHash = array::boolean_xor(CompositionHash, $old_total),\n");
             events.push_str("            TotalHash = array::boolean_xor(IntrinsicHash, array::boolean_xor(CompositionHash, $old_total))\n");
             events.push_str(&format!("        WHERE RecordId = $before.{} AND RecordId != NONE;\n\n", parent_field));
             
             // Add contribution to New Parent
             events.push_str("        UPDATE _spooky_data_hash SET\n");
             events.push_str("            CompositionHash = array::boolean_xor(CompositionHash, $new_total),\n");
             events.push_str("            TotalHash = array::boolean_xor(IntrinsicHash, array::boolean_xor(CompositionHash, $new_total))\n");
             events.push_str(&format!("        WHERE RecordId = $after.{} AND RecordId != NONE;\n", parent_field));
             events.push_str("    } END;\n\n");
        }

        // 6. Cascade Down (Incoming References)
        // Need to find OTHER tables that reference THIS table.
        // In TS logic: it checked regex patterns or descriptions like "Record ID of table: ..."
        // In Rust parser: we have `relationships` vector in TableSchema!
        // But `TableSchema.relationships` lists OUTGOING relationships.
        // We need INCOMING. 
        // We can iterate all other tables and check THEIR relationships to see if they point to `table_name`.
        
        let mut cascade_updates = String::new();
        
        for other_table_name in sorted_table_names.iter() {
             if *other_table_name == *table_name { continue; }
             
             let other_table = tables.get(*other_table_name).unwrap();
             
             for rel in &other_table.relationships {
                 // Check if this relationship points to the current table
                 if rel.related_table == **table_name {
                     // Check if this is NOT a parent relationship (to avoid double update or circles? logic from TS says so)
                     // "if (!isParentField)"
                     
                     let other_parent_field = parent_map.get(&other_table_name.to_lowercase());
                     let is_parent_link = if let Some(p_field) = other_parent_field {
                         p_field == &rel.field_name
                     } else {
                         false
                     };

                     if !is_parent_link {
                         // This is a reference - update the spooky hash of the REFERENCING record
                         // We update its CompositionHash and TotalHash by XORing with the delta
                         cascade_updates.push_str("        UPDATE _spooky_data_hash SET\n");
                         cascade_updates.push_str("            CompositionHash = array::boolean_xor(CompositionHash, $intrinsic_delta),\n");
                         cascade_updates.push_str("            TotalHash = array::boolean_xor(TotalHash, $intrinsic_delta)\n");
                         cascade_updates.push_str(&format!("        WHERE RecordId IN (SELECT value id FROM {} WHERE {} = $after.id);\n\n", other_table_name, rel.field_name));
                     }
                 }
             }
        }

        if !cascade_updates.is_empty() {
             events.push_str("    -- CASCADE DOWN (References)\n");
             events.push_str("    IF $old_hash_data.RecordId != NONE AND $new_intrinsic != $old_hash_data.IntrinsicHash THEN {\n");
             events.push_str("        LET $intrinsic_delta = array::boolean_xor($new_intrinsic, $old_hash_data.IntrinsicHash);\n");
             events.push_str(&cascade_updates);
             events.push_str("    } END;\n");
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
             events.push_str("            CompositionHash = array::boolean_xor(CompositionHash, $old_total),\n");
             events.push_str("            TotalHash = array::boolean_xor(IntrinsicHash, array::boolean_xor(CompositionHash, $old_total))\n");
             events.push_str(&format!("        WHERE RecordId = $before.{};\n", parent_field));
             events.push_str("    } END;\n\n");
        }

        events.push_str("    DELETE _spooky_data_hash WHERE RecordId = $before.id;\n");
        events.push_str("};\n\n");
    }

    events
}
