use regex::Regex;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct FieldAnnotation {
    pub name: String,
    pub value: Option<String>,
}

/// Known annotation names. Unknown annotations produce a warning.
const KNOWN_ANNOTATIONS: &[&str] = &["crdt", "parent"];

/// Extract field annotations from raw .surql content.
///
/// Must run BEFORE surrealdb-core parsing since that strips comments.
/// Returns: (table_name, field_name) → Vec<FieldAnnotation>
///
/// Supports two placements:
/// 1. Preceding-line: `-- @crdt text` on line(s) before DEFINE FIELD
/// 2. Trailing: `DEFINE FIELD ...; -- @crdt text` after the semicolon
///
/// Association rules:
/// - Blank lines clear pending annotations
/// - Non-annotation comments do NOT clear pending
/// - Annotations inside multi-line DEFINE FIELD bodies are ignored
/// - Unknown annotation names produce a warning (forward-compatible)
pub fn extract_field_annotations(
    content: &str,
) -> BTreeMap<(String, String), Vec<FieldAnnotation>> {
    let annotation_re = Regex::new(r"^--\s*@([a-z][a-z0-9_]*)(?:\s+(.+?))?\s*$").unwrap();
    let define_field_re =
        Regex::new(r"(?i)DEFINE\s+FIELD\s+(?:OVERWRITE\s+|IF\s+NOT\s+EXISTS\s+)?(\w+)\s+ON\s+(?:TABLE\s+)?(\w+)")
            .unwrap();

    let mut result: BTreeMap<(String, String), Vec<FieldAnnotation>> = BTreeMap::new();
    let mut pending: Vec<FieldAnnotation> = Vec::new();
    let mut in_define_field = false;
    let mut current_key: Option<(String, String)> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        // === Try to parse as standalone annotation comment ===
        if trimmed.starts_with("--") {
            if let Some(caps) = annotation_re.captures(trimmed) {
                let name = caps[1].to_string();
                let value = caps.get(2).map(|m| m.as_str().trim().to_string());

                if !KNOWN_ANNOTATIONS.contains(&name.as_str()) {
                    eprintln!(
                        "  ⚠ Unknown annotation @{} — known annotations: {}",
                        name,
                        KNOWN_ANNOTATIONS.join(", ")
                    );
                }

                let ann = FieldAnnotation { name, value };
                if !in_define_field {
                    pending.push(ann);
                }
                // Annotations inside multi-line DEFINE FIELD body are ignored
            }
            // Non-annotation comments don't clear pending
            continue;
        }

        // === Blank line: clear pending annotations ===
        if trimmed.is_empty() {
            pending.clear();
            continue;
        }

        // === DEFINE FIELD start ===
        if let Some(caps) = define_field_re.captures(trimmed) {
            let field = caps[1].to_string();
            let table = caps[2].to_string();
            let key = (table, field);

            // Attach pending preceding-line annotations
            if !pending.is_empty() {
                result
                    .entry(key.clone())
                    .or_default()
                    .extend(pending.drain(..));
            }

            // Check for trailing annotation after ';' on this line
            if let Some(semi_pos) = trimmed.rfind(';') {
                let after = trimmed[semi_pos + 1..].trim();
                if let Some(caps) = annotation_re.captures(after) {
                    let name = caps[1].to_string();
                    let value = caps.get(2).map(|m| m.as_str().trim().to_string());
                    result
                        .entry(key.clone())
                        .or_default()
                        .push(FieldAnnotation { name, value });
                }
                in_define_field = false;
                current_key = None;
            } else {
                // Multi-line statement — track until ';'
                in_define_field = true;
                current_key = Some(key);
            }
            continue;
        }

        // === Continuation of multi-line DEFINE FIELD ===
        if in_define_field {
            if let Some(semi_pos) = trimmed.rfind(';') {
                // Check for trailing annotation on the closing line
                let after = trimmed[semi_pos + 1..].trim();
                if let Some(caps) = annotation_re.captures(after) {
                    if let Some(key) = &current_key {
                        let name = caps[1].to_string();
                        let value = caps.get(2).map(|m| m.as_str().trim().to_string());
                        result
                            .entry(key.clone())
                            .or_default()
                            .push(FieldAnnotation { name, value });
                    }
                }
                in_define_field = false;
                current_key = None;
            }
            continue;
        }

        // === Any other non-empty line: clear pending ===
        pending.clear();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preceding_annotation() {
        let content = r#"
-- @crdt text
DEFINE FIELD content ON TABLE thread TYPE string;
"#;
        let result = extract_field_annotations(content);
        let anns = result.get(&("thread".to_string(), "content".to_string()));
        assert!(anns.is_some());
        let anns = anns.unwrap();
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].name, "crdt");
        assert_eq!(anns[0].value, Some("text".to_string()));
    }

    #[test]
    fn test_trailing_annotation() {
        let content = r#"DEFINE FIELD author ON TABLE thread TYPE record<user>; -- @parent"#;
        let result = extract_field_annotations(content);
        let anns = result.get(&("thread".to_string(), "author".to_string()));
        assert!(anns.is_some());
        let anns = anns.unwrap();
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].name, "parent");
        assert_eq!(anns[0].value, None);
    }

    #[test]
    fn test_multiline_with_trailing() {
        let content = r#"DEFINE FIELD content ON TABLE thread TYPE string
    ASSERT $value != NONE AND string::len($value) > 0; -- @crdt text"#;
        let result = extract_field_annotations(content);
        let anns = result.get(&("thread".to_string(), "content".to_string()));
        assert!(anns.is_some());
        assert_eq!(anns.unwrap()[0].name, "crdt");
    }

    #[test]
    fn test_blank_line_clears_pending() {
        let content = r#"
-- @crdt text

DEFINE FIELD content ON TABLE thread TYPE string;
"#;
        let result = extract_field_annotations(content);
        let anns = result.get(&("thread".to_string(), "content".to_string()));
        assert!(anns.is_none());
    }

    #[test]
    fn test_multiple_annotations() {
        let content = r#"
-- @crdt text
-- @parent
DEFINE FIELD content ON TABLE thread TYPE string;
"#;
        let result = extract_field_annotations(content);
        let anns = result.get(&("thread".to_string(), "content".to_string()));
        assert!(anns.is_some());
        assert_eq!(anns.unwrap().len(), 2);
    }

    #[test]
    fn test_no_false_positives() {
        let content = r#"
-- TODO: @crdt support for this field later
DEFINE FIELD content ON TABLE thread TYPE string;
"#;
        let result = extract_field_annotations(content);
        // "TODO: @crdt..." doesn't match the regex (text before @)
        let anns = result.get(&("thread".to_string(), "content".to_string()));
        assert!(anns.is_none());
    }

    #[test]
    fn test_non_annotation_comments_dont_clear_pending() {
        let content = r#"
-- @crdt text
-- This field stores the thread content
DEFINE FIELD content ON TABLE thread TYPE string;
"#;
        let result = extract_field_annotations(content);
        let anns = result.get(&("thread".to_string(), "content".to_string()));
        assert!(anns.is_some());
        assert_eq!(anns.unwrap().len(), 1);
        assert_eq!(anns.unwrap()[0].name, "crdt");
    }
}
