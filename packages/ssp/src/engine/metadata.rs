//! Metadata and versioning strategies.
//!
//! This module handles how versions are computed, stored, and retrieved
//! for different views (Optimistic, Explicit, HashBased).

use super::types::SpookyValue;
use super::types::FastMap;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::collections::HashMap;

pub type VersionMap = FastMap<SmolStr, u64>;

/// Strategy for computing record versions
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VersionStrategy {
    /// Auto-increment versions on every update (default for Streaming)
    #[default]
    Optimistic,
    /// Use explicit versions provided during ingestion
    Explicit,
    /// Derive version from content hash (for Tree/Flat consistency)
    HashBased,
    /// Do not track versions (stateless)
    None,
}

/// Metadata for a single record update
#[derive(Debug, Clone, Default)]
pub struct RecordMeta {
    pub version: Option<u64>,
    pub hash: Option<String>,
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
}

/// Metadata for a batch of updates
#[derive(Debug, Clone, Default)]
pub struct BatchMeta {
    pub records: FastMap<SmolStr, RecordMeta>,
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

    pub fn add_record(&mut self, id: impl Into<SmolStr>, meta: RecordMeta) -> &mut Self {
        self.records.insert(id.into(), meta);
        self
    }

    pub fn get(&self, id: &str) -> Option<&RecordMeta> {
        self.records.get(id)
    }
}

/// Store for content hashes
pub type HashStore = HashMap<SmolStr, String>;

/// Persistent state for view metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewMetadataState {
    pub versions: VersionMap,
    #[serde(default)]
    pub hashes: HashStore,
    pub strategy: VersionStrategy,
    #[serde(default)]
    pub last_result_hash: String,
}

impl Default for ViewMetadataState {
    fn default() -> Self {
        Self {
            versions: VersionMap::default(),
            hashes: HashStore::default(),
            strategy: VersionStrategy::default(),
            last_result_hash: String::new(),
        }
    }
}

impl ViewMetadataState {
    pub fn new(strategy: VersionStrategy) -> Self {
        Self {
            strategy,
            ..Default::default()
        }
    }

    #[inline]
    pub fn get_version(&self, id: &str) -> u64 {
        self.versions.get(id).copied().unwrap_or(0)
    }

    #[inline]
    pub fn set_version(&mut self, id: impl Into<SmolStr>, version: u64) {
        self.versions.insert(id.into(), version);
    }

    #[inline]
    pub fn remove(&mut self, id: &str) {
        self.versions.remove(id);
        self.hashes.remove(id);
    }

    #[inline]
    pub fn contains(&self, id: &str) -> bool {
        self.versions.contains_key(id)
    }

    pub fn is_first_run(&self) -> bool {
        self.last_result_hash.is_empty() && self.versions.is_empty()
    }
    
    // Performance optimization: Reserve capacity
    pub fn reserve(&mut self, additional: usize) {
        self.versions.reserve(additional);
    }
}

/// Helper struct for computing versions
#[derive(Debug)]
pub struct MetadataProcessor {
    pub strategy: VersionStrategy,
}

pub struct VersionResult {
    pub version: u64,
    pub changed: bool,
}

impl MetadataProcessor {
    pub fn new(strategy: VersionStrategy) -> Self {
        Self { strategy }
    }

    #[inline]
    pub fn compute_new_version(
        &self,
        _id: &str,
        current_version: u64,
        meta: Option<&RecordMeta>,
    ) -> VersionResult {
        match self.strategy {
            VersionStrategy::Optimistic => VersionResult {
                version: if current_version == 0 { 1 } else { current_version + 1 },
                changed: true,
            },
            VersionStrategy::Explicit => {
                let version = meta.and_then(|m| m.version).unwrap_or(current_version);
                VersionResult {
                    version,
                    changed: version != current_version,
                }
            },
            VersionStrategy::HashBased => {
                // For hash-based, we usually need the hash to compute version
                // but for new records, we might just default to 1 or explicit
                VersionResult { version: 1, changed: true }
            },
            VersionStrategy::None => VersionResult {
                version: 0,
                changed: false,
            },
        }
    }

    #[inline]
    pub fn compute_update_version(
        &self,
        _id: &str,
        current_version: u64,
        meta: Option<&RecordMeta>,
        is_optimistic: bool,
    ) -> VersionResult {
        match self.strategy {
            VersionStrategy::Optimistic => {
                if is_optimistic {
                     VersionResult {
                        version: current_version + 1,
                        changed: true,
                    }
                } else {
                    VersionResult {
                        version: current_version,
                        changed: false,
                    }
                }
            },
            VersionStrategy::Explicit => {
                if let Some(m) = meta {
                    if let Some(v) = m.version {
                         return VersionResult {
                            version: v,
                            changed: v != current_version,
                        };
                    }
                }
                // Fallback to current if not provided
                VersionResult {
                    version: current_version,
                    changed: false,
                }
            },
            VersionStrategy::HashBased => {
                // Logic usually handled by caller comparing hashes
                 VersionResult {
                    version: current_version + 1, // Placeholder
                    changed: true,
                }
            },
            VersionStrategy::None => VersionResult {
                version: 0,
                changed: false,
            },
        }
    }
}
