/// Create a Z-set key from a table name and record ID.
///
/// Format: "table:raw_id" (e.g., "users:1").
/// Strips the first colon-separated prefix from the ID to normalize it.
/// This handles SurrealDB-style record IDs like "user:1" where the prefix
/// is the record type.
pub fn make_key(table: &str, id: &str) -> String {
    let raw_id = id.split_once(':').map(|(_, rest)| rest).unwrap_or(id);
    format!("{table}:{raw_id}")
}

/// Extract the raw (stripped) portion of a record ID.
///
/// Strips the first colon-separated prefix, mirroring what `make_key` does.
pub fn raw_id(id: &str) -> &str {
    id.split_once(':').map(|(_, rest)| rest).unwrap_or(id)
}

/// Parse a Z-set key into (table, id).
///
/// Returns `None` if the key doesn't contain a ':'.
pub fn parse_key(key: &str) -> Option<(&str, &str)> {
    key.split_once(':')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_key_simple() {
        assert_eq!(make_key("user", "abc123"), "user:abc123");
    }

    #[test]
    fn make_key_strips_existing_prefix() {
        assert_eq!(make_key("user", "user:abc123"), "user:abc123");
    }

    #[test]
    fn parse_key_valid() {
        assert_eq!(parse_key("user:abc123"), Some(("user", "abc123")));
    }

    #[test]
    fn parse_key_no_colon() {
        assert_eq!(parse_key("nocolon"), None);
    }
}
