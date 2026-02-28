use crate::algebra::ZSet;
use crate::operator::QueryPlan;
use crate::types::SpookyValue;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Output format for a registered query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Flat,
    Tree,
    Streaming,
}

/// Materialized query output state.
///
/// Holds the accumulated output of a query's operator DAG.
/// The cache uses membership normalization (weights clamped to 0/1)
/// because the view output represents "what records are visible."
#[derive(Debug, Clone)]
pub struct View {
    pub query_id: String,
    /// The original query plan (kept for serialization/restore).
    pub plan: QueryPlan,
    /// Output cache: records currently in the materialized view.
    pub cache: ZSet,
    /// Hash of the last emitted output (for change detection).
    pub last_hash: String,
    pub format: OutputFormat,
    pub params: Option<SpookyValue>,
    /// Table names this query depends on.
    pub referenced_tables: Vec<String>,
    /// Tables referenced inside subquery projections (may overlap with primary tables).
    pub subquery_tables: Vec<String>,
    /// Monotonic counter bumped on subquery content changes, included in hash.
    pub content_generation: u64,
    /// Subquery record tracking: child_key → (parent_key, alias).
    /// Tracks which subquery records are visible through parent records in the view.
    pub subquery_cache: HashMap<String, (String, String)>,
}

impl View {
    pub fn new(
        query_id: String,
        plan: QueryPlan,
        format: OutputFormat,
        params: Option<SpookyValue>,
        referenced_tables: Vec<String>,
    ) -> Self {
        let subquery_tables = plan.root.subquery_tables();
        Self {
            query_id,
            plan,
            cache: HashMap::new(),
            last_hash: String::new(),
            format,
            params,
            referenced_tables,
            subquery_tables,
            content_generation: 0,
            subquery_cache: HashMap::new(),
        }
    }

    /// Apply a view delta to the cache with membership normalization.
    /// Positive weights → set to 1 (present).
    /// Zero or negative → remove (absent).
    pub fn apply_delta(&mut self, delta: &ZSet) {
        for (key, &weight_delta) in delta {
            let old = self.cache.get(key).copied().unwrap_or(0);
            let new_weight = old + weight_delta;
            if new_weight > 0 {
                self.cache.insert(key.clone(), 1);
            } else {
                self.cache.remove(key);
            }
        }
    }

    /// Compute a hash of the current cache state for change detection.
    /// Includes `content_generation` so the hash changes when subquery data changes.
    pub fn compute_hash(&self) -> String {
        let mut keys: Vec<&String> = self.cache.keys().collect();
        keys.sort();
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for key in &keys {
            key.hash(&mut hasher);
        }
        self.content_generation.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    /// Bump the content generation counter (called when subquery data changes).
    pub fn bump_content_generation(&mut self) {
        self.content_generation += 1;
    }
}
