//! Metadata strategies for version and hash computation.
//!
//! This module provides pluggable strategies for:
//! - Version computation (optimistic increment, explicit value, hash-based)
//! - Hash computation (for change detection)
//! - Source of truth management (version_map, hash_map, etc.)
//!
//! The goal is to keep view.rs focused on pure DBSP delta computation,
//! while this module handles all metadata/versioning concerns.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use super::types::{FastMap, SpookyValue};

/// Type alias for version storage
pub type VersionMap = FastMap<SmolStr, u64>;

/// Type alias for hash storage (for tree/hash-based strategies)
pub type HashStore = FastMap<SmolStr, String>;

/// Strategy for computing new versions
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VersionStrategy {
    /// Optimistic: auto-increment version on update (default)
    #[default]
    Optimistic,
    /// Explicit: version provided externally during ingestion
    Explicit,
    /// Hash-based: version derived from content hash
    HashBased,
    /// None: no version tracking (stateless)
    None,
}

/// Metadata for a single record change (passed during ingestion)
#[derive(Clone, Debug, Default)]
pub struct RecordMeta {
    /// Explicit version (used when VersionStrategy::Explicit)
    pub version: Option<u64>,
    /// Content hash (used when VersionStrategy::HashBased)
    pub hash: Option<String>,
    /// Custom metadata (extensible)
    pub custom: Option<SpookyValue>,
}

impl RecordMeta {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_version(mut self, version: u64) -> Self {
        self.version = Some(version);
        self
    }

    pub fn with_hash(mut self, hash: String) -> Self {
        self.hash = Some(hash);
        self
    }
}

/// Batch metadata for multiple records
#[derive(Clone, Debug, Default)]
pub struct BatchMeta {
    /// Per-record metadata indexed by record ID
    pub records: FastMap<SmolStr, RecordMeta>,
    /// Default strategy for records without explicit meta
    pub default_strategy: VersionStrategy,
}

impl BatchMeta {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_strategy(mut self, strategy: VersionStrategy) -> Self {
        self.default_strategy = strategy;
        self
    }

    pub fn add_record(mut self, id: impl Into<SmolStr>, meta: RecordMeta) -> Self {
        self.records.insert(id.into(), meta);
        self
    }

    pub fn get(&self, id: &str) -> Option<&RecordMeta> {
        self.records.get(id)
    }
}

/// Result of version computation for a record
#[derive(Clone, Debug)]
pub struct VersionResult {
    pub version: u64,
    pub changed: bool,
}

/// Metadata processor - handles version/hash computation
#[derive(Clone, Debug, Default)]
pub struct MetadataProcessor {
    strategy: VersionStrategy,
}

impl MetadataProcessor {
    pub fn new(strategy: VersionStrategy) -> Self {
        Self { strategy }
    }

    /// Compute version for a new record (entering the view)
    #[inline]
    pub fn compute_new_version(
        &self,
        _id: &str,
        current_version: u64,
        meta: Option<&RecordMeta>,
    ) -> VersionResult {
        match &self.strategy {
            VersionStrategy::Optimistic => {
                // New record starts at version 1 (or keeps existing if re-entering)
                let version = if current_version == 0 { 1 } else { current_version };
                VersionResult { version, changed: current_version == 0 }
            }
            VersionStrategy::Explicit => {
                // Use explicit version if provided, otherwise default to 1
                let version = meta.and_then(|m| m.version).unwrap_or(1);
                VersionResult { 
                    version, 
                    changed: version != current_version 
                }
            }
            VersionStrategy::HashBased => {
                // For hash-based, version is always 1 (hash is the real identifier)
                VersionResult { version: 1, changed: current_version == 0 }
            }
            VersionStrategy::None => {
                VersionResult { version: 0, changed: false }
            }
        }
    }

    /// Compute version for an updated record (already in view, content changed)
    #[inline]
    pub fn compute_update_version(
        &self,
        _id: &str,
        current_version: u64,
        meta: Option<&RecordMeta>,
        is_optimistic: bool,
    ) -> VersionResult {
        match &self.strategy {
            VersionStrategy::Optimistic => {
                if is_optimistic {
                    // Optimistic update: increment version
                    let new_version = current_version.saturating_add(1);
                    VersionResult { version: new_version, changed: true }
                } else {
                    // Non-optimistic: keep version (remote sync scenario)
                    VersionResult { version: current_version, changed: false }
                }
            }
            VersionStrategy::Explicit => {
                // Use explicit version if provided
                if let Some(meta) = meta {
                    if let Some(explicit_ver) = meta.version {
                        return VersionResult { 
                            version: explicit_ver, 
                            changed: explicit_ver != current_version 
                        };
                    }
                }
                // No explicit version provided, keep current
                VersionResult { version: current_version, changed: false }
            }
            VersionStrategy::HashBased => {
                // Hash-based: version changes if hash changes (handled externally)
                // Here we just report that change detection should use hash
                VersionResult { version: current_version, changed: false }
            }
            VersionStrategy::None => {
                VersionResult { version: 0, changed: false }
            }
        }
    }

    /// Check if a record has changed based on strategy
    #[inline]
    pub fn has_changed(
        &self,
        _id: &str,
        old_hash: Option<&str>,
        new_hash: Option<&str>,
        old_version: u64,
        new_version: u64,
    ) -> bool {
        match &self.strategy {
            VersionStrategy::HashBased => {
                // Compare hashes
                match (old_hash, new_hash) {
                    (Some(old), Some(new)) => old != new,
                    (None, Some(_)) => true,  // New hash = changed
                    (Some(_), None) => true,  // Lost hash = changed
                    (None, None) => false,
                }
            }
            _ => {
                // Version-based comparison
                old_version != new_version
            }
        }
    }
}

/// View metadata state - persisted with the view
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ViewMetadataState {
    /// Version map (record_id -> version)
    #[serde(default)]
    pub versions: VersionMap,
    
    /// Hash map (record_id -> content_hash) - for HashBased strategy
    #[serde(default, skip_serializing_if = "FastMap::is_empty")]
    pub hashes: HashStore,
    
    /// Strategy in use
    #[serde(default)]
    pub strategy: VersionStrategy,
    
    /// Last computed result hash (for change detection)
    #[serde(default)]
    pub last_result_hash: String,
}

impl ViewMetadataState {
    pub fn new(strategy: VersionStrategy) -> Self {
        Self {
            versions: FastMap::default(),
            hashes: FastMap::default(),
            strategy,
            last_result_hash: String::new(),
        }
    }

    /// Check if this is the first run (no data yet)
    #[inline]
    pub fn is_first_run(&self) -> bool {
        self.last_result_hash.is_empty()
    }

    /// Get version for a record
    #[inline]
    pub fn get_version(&self, id: &str) -> u64 {
        self.versions.get(id).copied().unwrap_or(0)
    }

    /// Set version for a record
    #[inline]
    pub fn set_version(&mut self, id: impl Into<SmolStr>, version: u64) {
        self.versions.insert(id.into(), version);
    }

    /// Remove a record from tracking
    #[inline]
    pub fn remove(&mut self, id: &str) {
        let key = SmolStr::new(id);
        self.versions.remove(&key);
        self.hashes.remove(&key);
    }

    /// Check if a record is being tracked
    #[inline]
    pub fn contains(&self, id: &str) -> bool {
        self.versions.contains_key(id)
    }

    /// Get hash for a record (HashBased strategy)
    #[inline]
    pub fn get_hash(&self, id: &str) -> Option<&String> {
        self.hashes.get(id)
    }

    /// Set hash for a record (HashBased strategy)
    #[inline]
    pub fn set_hash(&mut self, id: impl Into<SmolStr>, hash: String) {
        self.hashes.insert(id.into(), hash);
    }

    /// Reserve capacity for expected number of records
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.versions.reserve(additional);
        if matches!(self.strategy, VersionStrategy::HashBased) {
            self.hashes.reserve(additional);
        }
    }

    /// Get all tracked record IDs
    pub fn tracked_ids(&self) -> impl Iterator<Item = &SmolStr> {
        self.versions.keys()
    }

    /// Clear all tracking data
    pub fn clear(&mut self) {
        self.versions.clear();
        self.hashes.clear();
        self.last_result_hash.clear();
    }
}

/// Builder for ViewMetadataState
pub struct MetadataStateBuilder {
    state: ViewMetadataState,
}

impl MetadataStateBuilder {
    pub fn new() -> Self {
        Self {
            state: ViewMetadataState::default(),
        }
    }

    pub fn with_strategy(mut self, strategy: VersionStrategy) -> Self {
        self.state.strategy = strategy;
        self
    }

    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.state.reserve(capacity);
        self
    }

    pub fn build(self) -> ViewMetadataState {
        self.state
    }
}

impl Default for MetadataStateBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_optimistic_new_version() {
        let processor = MetadataProcessor::new(VersionStrategy::Optimistic);
        
        // New record (version 0 -> 1)
        let result = processor.compute_new_version("test:1", 0, None);
        assert_eq!(result.version, 1);
        assert!(result.changed);
        
        // Re-entering record keeps version
        let result = processor.compute_new_version("test:1", 5, None);
        assert_eq!(result.version, 5);
        assert!(!result.changed);
    }

    #[test]
    fn test_optimistic_update_version() {
        let processor = MetadataProcessor::new(VersionStrategy::Optimistic);
        
        // Optimistic update increments
        let result = processor.compute_update_version("test:1", 5, None, true);
        assert_eq!(result.version, 6);
        assert!(result.changed);
        
        // Non-optimistic keeps version
        let result = processor.compute_update_version("test:1", 5, None, false);
        assert_eq!(result.version, 5);
        assert!(!result.changed);
    }

    #[test]
    fn test_explicit_version() {
        let processor = MetadataProcessor::new(VersionStrategy::Explicit);
        let meta = RecordMeta::new().with_version(42);
        
        let result = processor.compute_new_version("test:1", 0, Some(&meta));
        assert_eq!(result.version, 42);
        assert!(result.changed);
    }

    #[test]
    fn test_metadata_state() {
        let mut state = ViewMetadataState::new(VersionStrategy::Optimistic);
        
        assert!(state.is_first_run());
        
        state.set_version("test:1", 5);
        assert_eq!(state.get_version("test:1"), 5);
        assert!(state.contains("test:1"));
        
        state.remove("test:1");
        assert!(!state.contains("test:1"));
        assert_eq!(state.get_version("test:1"), 0);
    }
}
