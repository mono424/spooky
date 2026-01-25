use rustc_hash::FxHasher;
use smol_str::SmolStr;
use std::hash::BuildHasherDefault;

pub type Weight = i64;
pub type RowKey = SmolStr;
pub type FastMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FxHasher>>;
pub type ZSet = FastMap<RowKey, Weight>;
pub type VersionMap = FastMap<SmolStr, u64>;

/// Create a ZSet key from table name and record ID
/// 
/// # Arguments
/// * `table` - Table name (e.g., "user")
/// * `id` - Raw record ID WITHOUT table prefix (e.g., "xyz123")
/// 
/// # Returns
/// * ZSet key in format "table:id" (e.g., "user:xyz123")
#[inline]
pub fn make_zset_key(table: &str, id: &str) -> SmolStr {
    // Strip any existing table prefix from id
    let raw_id = id.split_once(':').map(|(_, rest)| rest).unwrap_or(id);
    
    let combined_len = table.len() + 1 + raw_id.len();
    if combined_len <= 23 {
        // SmolStr inline storage optimization
        let mut buf = String::with_capacity(combined_len);
        buf.push_str(table);
        buf.push(':');
        buf.push_str(raw_id);
        SmolStr::new(buf)
    } else {
        SmolStr::new(format!("{}:{}", table, raw_id))
    }
}

/// Extract table and raw ID from a ZSet key
#[inline]
pub fn parse_zset_key(key: &str) -> Option<(&str, &str)> {
    key.split_once(':')
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_make_zset_key_simple() {
        assert_eq!(make_zset_key("user", "xyz123").as_str(), "user:xyz123");
    }
    
    #[test]
    fn test_make_zset_key_strips_prefix() {
        // If id already has prefix, strip it
        assert_eq!(make_zset_key("user", "user:xyz123").as_str(), "user:xyz123");
    }
    
    #[test]
    fn test_parse_zset_key() {
        assert_eq!(parse_zset_key("user:xyz123"), Some(("user", "xyz123")));
    }
}
