use crate::annotations::has_annotation;
use crate::backend::DeployMode;
use crate::parser::{FieldType, TableSchema};
use std::collections::BTreeMap;

/// Generate Sp00ky events for data hashing and graph synchronization
// ... imports ...

/// Generate Sp00ky events for data hashing and graph synchronization
pub fn generate_sp00ky_events(
    tables: &BTreeMap<String, TableSchema>,
    _raw_content: &str,
    is_client: bool,
    mode: &DeployMode,
    _endpoint: Option<&str>,
    _secret: Option<&str>,
) -> String {
    // 2. Generate Events
    let mut events = String::from("\n-- ==================================================\n-- AUTO-GENERATED SP00KY EVENTS\n-- ==================================================\n\n");

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
            // Skip system/internal tables and the sp00ky hash tables themselves
            if table_name.starts_with("_00_") {
                continue;
            }

            let table = tables.get(*table_name).unwrap();

            if table.is_relation {
                continue;
            }

            // @nosync tables never sync: emit no events for them.
            if table.no_sync {
                continue;
            }

            // --------------------------------------------------
            // A. Client Mutation Event
            // --------------------------------------------------
            events.push_str(&format!("-- Table: {} Client Mutation\n", table_name));
            events.push_str(&format!(
                "DEFINE EVENT OVERWRITE _00_{}_client_mutation ON TABLE {}\n",
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
                "DEFINE EVENT OVERWRITE _00_{}_client_delete ON TABLE {}\n",
                table_name, table_name
            ));
            events.push_str("WHEN $event = \"DELETE\"\nTHEN {\n");
            events.push_str("    -- No-op for now.\n");
            events.push_str("};\n\n");
        }

        return events;
    }

    // Remote Logic: DBSP Ingest (Surrealism) OR Sidecar HTTP Call

    let is_http = *mode == DeployMode::Singlenode || *mode == DeployMode::Cluster;

    // Sort table names for deterministic output
    let mut sorted_table_names: Vec<_> = tables.keys().collect();
    sorted_table_names.sort();

    for table_name in &sorted_table_names {
        // Skip system/internal tables and the sp00ky hash tables themselves
        if table_name.starts_with("_00_") {
            continue;
        }

        let table = tables.get(*table_name).unwrap();

        // Skip relation tables that are explicitly marked as such (if we had that metadata easily available)
        // In the parser, we store is_relation.
        if table.is_relation {
            continue;
        }

        // @nosync tables never sync: emit no events for them, so SurrealDB
        // posts no ingest to the scheduler/SSP.
        if table.no_sync {
            continue;
        }

        // ===================================
        // 1. MUTATION EVENT (CREATE / UPDATE)
        // ===================================
        // Merges version tracking and data ingestion
        events.push_str(&format!(
            "DEFINE EVENT OVERWRITE _00_{}_mutation ON TABLE {}\n",
            table_name, table_name
        ));
        events.push_str("WHEN $before != $after AND $event != \"DELETE\"\nTHEN {\n");

        // --- Versioning Logic ---
        events.push_str("    LET $sp00ky_ver_rec = IF $event = \"CREATE\" {\n");
        events.push_str(
            "        (CREATE _00_version SET record_id = $after.id, version = 1 RETURN AFTER)\n",
        );
        events.push_str("    } ELSE IF $event = \"UPDATE\" {\n");
        events.push_str("        IF $sp00ky_target_version != NONE AND $sp00ky_target_version.id == $after.id {\n");
        events.push_str("            LET $u = (UPDATE _00_version SET version = <int>$sp00ky_target_version.version WHERE record_id = $after.id RETURN AFTER);\n");
        events.push_str("            LET $sp00ky_target_version = NONE;\n");
        events.push_str("            $u\n");
        events.push_str("        } ELSE {\n");
        events.push_str("            (UPDATE _00_version SET version += 1 WHERE record_id = $after.id RETURN AFTER)\n");
        events.push_str("        }\n");
        events.push_str("    };\n");
        events.push_str("    LET $sp00ky_ver = $sp00ky_ver_rec[0].version;\n\n");

        // --- Ingestion Logic ---
        events.push_str("    LET $plain_after = {\n");
        events.push_str("        id: <string>($after.id OR \"\"),\n");

        let mut all_fields: Vec<_> = table.fields.keys().collect();
        all_fields.sort();

        for field_name in all_fields {
            let field_def = table.fields.get(field_name).unwrap();
            // Skip @crdt fields from the ingest payload. Their value is a
            // raw LoroDoc snapshot (TYPE bytes) and JSON has no native
            // bytes encoding, so SurrealDB's http::post would either drop
            // the field or shape-shift it depending on transport. The SSP
            // doesn't filter or join on CRDT contents anyway — its job is
            // membership tracking — so omitting them keeps the payload
            // clean and saves bandwidth on every keystroke debounce push.
            if has_annotation(&field_def.annotations, "crdt") {
                continue;
            }
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
        events.push_str("        _00_rv: (SELECT VALUE version FROM ONLY _00_version WHERE record_id = $after.id)\n");
        events.push_str("    };\n");

        if is_http {
            events.push_str("    LET $payload = {\n");
            events.push_str(&format!("        table: '{}',\n", table_name));
            events.push_str("        op: $event,\n");
            events.push_str("        id: <string>($after.id OR \"\"),\n");
            events.push_str("        record: $plain_after,\n");
            events.push_str("        hash: \"\"\n");
            events.push_str("    };\n");

            events.push_str("    http::post($sp00ky_endpoint + '/ingest', $payload, { \"Authorization\": \"Bearer \" + $sp00ky_secret });\n");
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
            "DEFINE EVENT OVERWRITE _00_{}_delete ON TABLE {}\n",
            table_name, table_name
        ));
        events.push_str("WHEN $event = \"DELETE\"\nTHEN {\n");

        // --- Versioning Logic ---
        events.push_str("    DELETE _00_version WHERE record_id = $before.id;\n\n");
        // CRDT and cursor state live inline on the parent row itself, so
        // there is no sidecar table to clean up here — the row deletion
        // takes the snapshot with it.

        // --- Ingestion Logic ---
        events.push_str("    LET $plain_before = {\n");
        events.push_str("        id: <string>($before.id OR \"\"),\n");

        let mut all_fields_del: Vec<_> = table.fields.keys().collect();
        all_fields_del.sort();

        for field_name in all_fields_del {
            let field_def = table.fields.get(field_name).unwrap();
            // See the matching skip in the mutation event above for why
            // CRDT bytes don't go through the JSON ingest payload.
            if has_annotation(&field_def.annotations, "crdt") {
                continue;
            }
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

        if is_http {
            events.push_str("    LET $payload = {\n");
            events.push_str(&format!("        table: '{}',\n", table_name));
            events.push_str("        op: \"DELETE\",\n");
            events.push_str("        id: <string>($before.id OR \"\"),\n");
            events.push_str("        record: $plain_before,\n");
            events.push_str("        hash: \"\"\n");
            events.push_str("    };\n");

            events.push_str("    http::post($sp00ky_endpoint + '/ingest', $payload, { \"Authorization\": \"Bearer \" + $sp00ky_secret });\n");
        } else {
            events.push_str(&format!("    mod::dbsp::ingest('{}', \"DELETE\", <string>($before.id OR \"\"), $plain_before);\n", table_name));
            events.push_str("    mod::dbsp::save_state(NONE);\n");
        }
        events.push_str("};\n\n");
    }

    // ===================================================================
    // _00_user_feature (feature-flag assignments)
    // ===================================================================
    // Feature-flag assignments are written by the scheduler sweep and the
    // `spky flag` CLI under the project root token, never through the client
    // up-queue. Without an ingest-notify event the SSP never learns of the
    // change, so a client already subscribed to its flag would not see a new
    // variant until it re-registered. Emit the same mutation/delete events
    // every app table gets (skipped above for `_00_` tables) so a root UPSERT
    // reaches `/ingest`, the SSP recomputes the registered query, and the
    // subscriber's `_00_list_ref_user_<id>` updates in real time.
    //
    // Server-written only, so there is no client-mutation version targeting
    // (`$sp00ky_target_version`); the version simply increments.
    events.push_str("-- Table: _00_user_feature Mutation (server-written; ingest-notify)\n");
    events.push_str("DEFINE EVENT OVERWRITE _00_user_feature_mutation ON TABLE _00_user_feature\n");
    events.push_str("WHEN $before != $after AND $event != \"DELETE\"\nTHEN {\n");
    events.push_str("    LET $sp00ky_ver_rec = IF $event = \"CREATE\" {\n");
    events.push_str(
        "        (CREATE _00_version SET record_id = $after.id, version = 1 RETURN AFTER)\n",
    );
    events.push_str("    } ELSE {\n");
    events.push_str(
        "        (UPDATE _00_version SET version += 1 WHERE record_id = $after.id RETURN AFTER)\n",
    );
    events.push_str("    };\n");
    events.push_str("    LET $plain_after = {\n");
    events.push_str("        id: <string>($after.id OR \"\"),\n");
    events.push_str("        user: <string>($after.user OR \"\"),\n");
    events.push_str("        key: $after.key,\n");
    events.push_str("        variant: $after.variant,\n");
    events.push_str("        payload: $after.payload,\n");
    events.push_str("        evaluated_at: <string>($after.evaluated_at OR \"\"),\n");
    events.push_str("        _00_rv: (SELECT VALUE version FROM ONLY _00_version WHERE record_id = $after.id)\n");
    events.push_str("    };\n");
    if is_http {
        events.push_str("    LET $payload = {\n");
        events.push_str("        table: '_00_user_feature',\n");
        events.push_str("        op: $event,\n");
        events.push_str("        id: <string>($after.id OR \"\"),\n");
        events.push_str("        record: $plain_after,\n");
        events.push_str("        hash: \"\"\n");
        events.push_str("    };\n");
        events.push_str("    http::post($sp00ky_endpoint + '/ingest', $payload, { \"Authorization\": \"Bearer \" + $sp00ky_secret });\n");
    } else {
        events.push_str("    mod::dbsp::ingest('_00_user_feature', $event, <string>($after.id OR \"\"), $plain_after);\n");
        events.push_str("    mod::dbsp::save_state(NONE);\n");
    }
    events.push_str("};\n\n");

    events.push_str("-- Table: _00_user_feature Deletion (ingest-notify)\n");
    events.push_str("DEFINE EVENT OVERWRITE _00_user_feature_delete ON TABLE _00_user_feature\n");
    events.push_str("WHEN $event = \"DELETE\"\nTHEN {\n");
    events.push_str("    DELETE _00_version WHERE record_id = $before.id;\n");
    events.push_str("    LET $plain_before = {\n");
    events.push_str("        id: <string>($before.id OR \"\"),\n");
    events.push_str("        user: <string>($before.user OR \"\"),\n");
    events.push_str("        key: $before.key,\n");
    events.push_str("        variant: $before.variant,\n");
    events.push_str("        payload: $before.payload,\n");
    events.push_str("        evaluated_at: <string>($before.evaluated_at OR \"\")\n");
    events.push_str("    };\n");
    if is_http {
        events.push_str("    LET $payload = {\n");
        events.push_str("        table: '_00_user_feature',\n");
        events.push_str("        op: \"DELETE\",\n");
        events.push_str("        id: <string>($before.id OR \"\"),\n");
        events.push_str("        record: $plain_before,\n");
        events.push_str("        hash: \"\"\n");
        events.push_str("    };\n");
        events.push_str("    http::post($sp00ky_endpoint + '/ingest', $payload, { \"Authorization\": \"Bearer \" + $sp00ky_secret });\n");
    } else {
        events.push_str("    mod::dbsp::ingest('_00_user_feature', \"DELETE\", <string>($before.id OR \"\"), $plain_before);\n");
        events.push_str("    mod::dbsp::save_state(NONE);\n");
    }
    events.push_str("};\n\n");

    // ===================================================================
    // _00_app_release (per-frontend current-version announcements)
    // ===================================================================
    // Written root-only by `spky deploy` / `spky release` / the git-linked
    // builder, never through the client up-queue - same situation as
    // _00_user_feature above, so it needs the same explicit ingest-notify
    // events for a row change to reach already-subscribed clients live.
    events.push_str("-- Table: _00_app_release Mutation (server-written; ingest-notify)\n");
    events.push_str("DEFINE EVENT OVERWRITE _00_app_release_mutation ON TABLE _00_app_release\n");
    events.push_str("WHEN $before != $after AND $event != \"DELETE\"\nTHEN {\n");
    events.push_str("    LET $sp00ky_ver_rec = IF $event = \"CREATE\" {\n");
    events.push_str(
        "        (CREATE _00_version SET record_id = $after.id, version = 1 RETURN AFTER)\n",
    );
    events.push_str("    } ELSE {\n");
    events.push_str(
        "        (UPDATE _00_version SET version += 1 WHERE record_id = $after.id RETURN AFTER)\n",
    );
    events.push_str("    };\n");
    events.push_str("    LET $plain_after = {\n");
    events.push_str("        id: <string>($after.id OR \"\"),\n");
    events.push_str("        app: $after.app,\n");
    events.push_str("        version: $after.version,\n");
    events.push_str("        cache_bust: $after.cache_bust,\n");
    events.push_str("        mandatory: $after.mandatory,\n");
    events.push_str("        released_at: <string>($after.released_at OR \"\"),\n");
    events.push_str("        _00_rv: (SELECT VALUE version FROM ONLY _00_version WHERE record_id = $after.id)\n");
    events.push_str("    };\n");
    if is_http {
        events.push_str("    LET $payload = {\n");
        events.push_str("        table: '_00_app_release',\n");
        events.push_str("        op: $event,\n");
        events.push_str("        id: <string>($after.id OR \"\"),\n");
        events.push_str("        record: $plain_after,\n");
        events.push_str("        hash: \"\"\n");
        events.push_str("    };\n");
        events.push_str("    http::post($sp00ky_endpoint + '/ingest', $payload, { \"Authorization\": \"Bearer \" + $sp00ky_secret });\n");
    } else {
        events.push_str("    mod::dbsp::ingest('_00_app_release', $event, <string>($after.id OR \"\"), $plain_after);\n");
        events.push_str("    mod::dbsp::save_state(NONE);\n");
    }
    events.push_str("};\n\n");

    events.push_str("-- Table: _00_app_release Deletion (ingest-notify)\n");
    events.push_str("DEFINE EVENT OVERWRITE _00_app_release_delete ON TABLE _00_app_release\n");
    events.push_str("WHEN $event = \"DELETE\"\nTHEN {\n");
    events.push_str("    DELETE _00_version WHERE record_id = $before.id;\n");
    events.push_str("    LET $plain_before = {\n");
    events.push_str("        id: <string>($before.id OR \"\"),\n");
    events.push_str("        app: $before.app,\n");
    events.push_str("        version: $before.version,\n");
    events.push_str("        cache_bust: $before.cache_bust,\n");
    events.push_str("        mandatory: $before.mandatory,\n");
    events.push_str("        released_at: <string>($before.released_at OR \"\")\n");
    events.push_str("    };\n");
    if is_http {
        events.push_str("    LET $payload = {\n");
        events.push_str("        table: '_00_app_release',\n");
        events.push_str("        op: \"DELETE\",\n");
        events.push_str("        id: <string>($before.id OR \"\"),\n");
        events.push_str("        record: $plain_before,\n");
        events.push_str("        hash: \"\"\n");
        events.push_str("    };\n");
        events.push_str("    http::post($sp00ky_endpoint + '/ingest', $payload, { \"Authorization\": \"Bearer \" + $sp00ky_secret });\n");
    } else {
        events.push_str("    mod::dbsp::ingest('_00_app_release', \"DELETE\", <string>($before.id OR \"\"), $plain_before);\n");
        events.push_str("    mod::dbsp::save_state(NONE);\n");
    }
    events.push_str("};\n\n");

    // ===================================================================
    // _00_heartbeat (e2e sync-pipeline probe)
    // ===================================================================
    // The scheduler's heartbeat loop UPSERTs `_00_heartbeat:probe` and then
    // polls each SSP for the last hb_seq it saw. This event is the first hop:
    // without it a probe write never leaves the database and the loop
    // measures nothing. `_00_` tables are skipped by the generator above, so
    // it is hand-written like _00_user_feature — minus the `_00_version`
    // machinery, because nothing subscribes to this row (the SSP just
    // records the seq in memory; the row is never client-synced).
    events.push_str("-- Table: _00_heartbeat Mutation (probe-written; ingest-notify)\n");
    events.push_str("DEFINE EVENT OVERWRITE _00_heartbeat_mutation ON TABLE _00_heartbeat\n");
    events.push_str("WHEN $before != $after AND $event != \"DELETE\"\nTHEN {\n");
    events.push_str("    LET $plain_after = {\n");
    events.push_str("        id: <string>($after.id OR \"\"),\n");
    events.push_str("        hb_seq: $after.hb_seq,\n");
    events.push_str("        sent_at: <string>($after.sent_at OR \"\")\n");
    events.push_str("    };\n");
    if is_http {
        events.push_str("    LET $payload = {\n");
        events.push_str("        table: '_00_heartbeat',\n");
        events.push_str("        op: $event,\n");
        events.push_str("        id: <string>($after.id OR \"\"),\n");
        events.push_str("        record: $plain_after,\n");
        events.push_str("        hash: \"\"\n");
        events.push_str("    };\n");
        events.push_str("    http::post($sp00ky_endpoint + '/ingest', $payload, { \"Authorization\": \"Bearer \" + $sp00ky_secret });\n");
    } else {
        events.push_str("    mod::dbsp::ingest('_00_heartbeat', $event, <string>($after.id OR \"\"), $plain_after);\n");
        events.push_str("    mod::dbsp::save_state(NONE);\n");
    }
    events.push_str("};\n\n");

    events
}

#[cfg(test)]
mod tests {
    use super::*;

    // An empty table map isolates the always-emitted system-table events
    // (the user-table loop produces nothing), so these assertions target the
    // `_00_user_feature` ingest-notify events specifically.
    fn gen(is_client: bool, mode: DeployMode) -> String {
        generate_sp00ky_events(&BTreeMap::new(), "", is_client, &mode, None, None)
    }

    #[test]
    fn user_feature_events_post_to_ingest_in_http_modes() {
        for mode in [DeployMode::Singlenode, DeployMode::Cluster] {
            let out = gen(false, mode);
            assert!(
                out.contains(
                    "DEFINE EVENT OVERWRITE _00_user_feature_mutation ON TABLE _00_user_feature"
                ),
                "missing mutation event"
            );
            assert!(
                out.contains(
                    "DEFINE EVENT OVERWRITE _00_user_feature_delete ON TABLE _00_user_feature"
                ),
                "missing delete event"
            );
            assert!(
                out.contains("table: '_00_user_feature'"),
                "missing ingest payload table"
            );
            assert!(
                out.contains("http::post($sp00ky_endpoint + '/ingest'"),
                "feature-flag changes must notify the SSP ingest endpoint"
            );
        }
    }

    #[test]
    fn heartbeat_event_posts_to_ingest_in_http_modes() {
        for mode in [DeployMode::Singlenode, DeployMode::Cluster] {
            let out = gen(false, mode);
            assert!(
                out.contains(
                    "DEFINE EVENT OVERWRITE _00_heartbeat_mutation ON TABLE _00_heartbeat"
                ),
                "missing heartbeat mutation event"
            );
            assert!(
                out.contains("table: '_00_heartbeat'"),
                "missing heartbeat ingest payload table"
            );
        }
        // Surrealism mode routes through the module instead.
        let out = gen(false, DeployMode::Surrealism);
        assert!(
            out.contains("mod::dbsp::ingest('_00_heartbeat'"),
            "surrealism mode must route heartbeat through mod::dbsp"
        );
    }

    #[test]
    fn user_feature_events_use_dbsp_in_surrealism_mode() {
        let out = gen(false, DeployMode::Surrealism);
        assert!(
            out.contains("mod::dbsp::ingest('_00_user_feature'"),
            "missing dbsp ingest"
        );
        assert!(
            !out.contains("http::post"),
            "surrealism mode must not emit http::post"
        );
    }

    #[test]
    fn user_feature_events_are_remote_only_not_client() {
        // The client schema must not carry server-side ingest events for the
        // root-written assignments table.
        let out = gen(true, DeployMode::Singlenode);
        assert!(
            !out.contains("_00_user_feature_mutation"),
            "client schema must not emit _00_user_feature ingest events"
        );
    }

    #[test]
    fn nosync_table_emits_no_events() {
        use crate::parser::SchemaParser;
        let schema = r#"
DEFINE TABLE public SCHEMALESS;
DEFINE FIELD name ON TABLE public TYPE string;

-- @nosync
DEFINE TABLE secrets SCHEMALESS;
DEFINE FIELD token ON TABLE secrets TYPE string;
"#;
        let mut parser = SchemaParser::new();
        parser.parse_file(schema).unwrap();
        assert!(
            parser.tables["secrets"].no_sync,
            "secrets must be marked no_sync"
        );

        for is_client in [false, true] {
            let out = generate_sp00ky_events(
                &parser.tables,
                schema,
                is_client,
                &DeployMode::Singlenode,
                None,
                None,
            );
            assert!(
                out.contains("ON TABLE public"),
                "public table must get events"
            );
            assert!(
                !out.contains("ON TABLE secrets"),
                "@nosync table must not get events (is_client={is_client})"
            );
            assert!(
                !out.contains("table: 'secrets'"),
                "@nosync table must not appear in any ingest payload"
            );
        }
    }
}
