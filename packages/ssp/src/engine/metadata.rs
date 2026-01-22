//! Metadata and versioning strategies.
//!
//! This module handles how versions are computed, stored, and retrieved
//! for different views (Optimistic, Explicit, HashBased).

use super::types::SpookyValue;
use super::types::FastMap;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

pub type VersionMap = FastMap<SmolStr, u64>;

/// Strategy for computing record versions
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum IngestStrategy {
    /// Auto-increment versions on every update (default for Streaming)
    Optimistic,
    /// Use explicit versions provided during ingestion
    Explicit,
    /// Derive version from content hash (for Tree/Flat consistency)
    HashBased,
    /// Do not track versions (stateless)
    #[default]
    None,
}

/// Metadata for a single record update
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RecordMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom: Option<SpookyValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strategy: Option<IngestStrategy>,
}

impl RecordMeta {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_version(mut self, version: u64) -> Self {
        self.version = Some(version);
        self
    }

    pub fn with_strategy(mut self, strategy: IngestStrategy) -> Self {
        self.strategy = Some(strategy);
        self
    }

    pub fn with_custom(mut self, custom: SpookyValue) -> Self {
        self.custom = Some(custom);
        self
    }
}

/// Metadata for a batch of updates
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BatchMeta {
    pub records: FastMap<SmolStr, RecordMeta>,
    pub default_strategy: IngestStrategy,
}

impl BatchMeta {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_strategy(mut self, strategy: IngestStrategy) -> Self {
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
pub type HashStore = FastMap<SmolStr, String>;

/// Persistent state for view metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewMetadataState {
    pub versions: VersionMap,
    #[serde(default, skip_serializing_if = "FastMap::is_empty")]
    pub hashes: HashStore,
    pub strategy: IngestStrategy,
}

impl Default for ViewMetadataState {
    fn default() -> Self {
        Self {
            versions: VersionMap::default(),
            hashes: HashStore::default(),
            strategy: IngestStrategy::default(),
        }
    }
}

impl ViewMetadataState {
    #[inline]
    pub fn new(strategy: IngestStrategy) -> Self {
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
    pub fn set_versions_batch(&mut self, items: impl IntoIterator<Item = (SmolStr, u64)>) {
        for (id, version) in items {
            self.versions.insert(id, version);
        }
    }

    #[inline]
    pub fn remove(&mut self, id: &str) {
        self.versions.remove(id);
        self.hashes.remove(id);
    }

    #[inline]
    pub fn remove_batch(&mut self, ids: impl IntoIterator<Item = impl AsRef<str>>) {
        for id in ids {
            self.versions.remove(id.as_ref());
            self.hashes.remove(id.as_ref());
        }
    }

    #[inline]
    /// Check if version is tracked (NOT a membership check - use View.contains() for that)
    #[deprecated(note = "Use View.contains() for membership checks. This only checks version tracking.")]
    pub fn contains(&self, id: &str) -> bool {
        self.versions.contains_key(id)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.versions.is_empty()
    }

    #[inline]
    pub fn is_first_run(&self) -> bool {
        self.versions.is_empty()
    }
    
    // Performance optimization: Reserve capacity
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.versions.reserve(additional);
    }
}

/// Helper struct for computing versions
#[derive(Debug)]
pub struct MetadataProcessor {
    pub strategy: IngestStrategy,
}

pub struct VersionResult {
    pub version: u64,
    pub changed: bool,
}

impl MetadataProcessor {
    #[inline]
    pub fn new(strategy: IngestStrategy) -> Self {
        Self { strategy }
    }

    #[inline]
    pub fn compute_new_version(
        &self,
        _id: &str,
        current_version: u64,
        meta: Option<&RecordMeta>,
    ) -> VersionResult {
        let strategy = meta.and_then(|m| m.strategy.as_ref()).unwrap_or(&self.strategy);
        match strategy {
            IngestStrategy::Optimistic => {
                let base = meta.and_then(|m| m.version).unwrap_or(current_version);
                VersionResult {
                    version: if base == 0 { 1 } else { base + 1 },
                    changed: true,
                }
            },
            IngestStrategy::Explicit => {
                let version = meta.and_then(|m| m.version).unwrap_or(current_version);
                VersionResult {
                    version,
                    changed: version != current_version,
                }
            },
            IngestStrategy::HashBased => {
                // For hash-based, we usually need the hash to compute version
                // but for new records, we might just default to 1 or explicit
                VersionResult { version: 1, changed: true }
            },
            IngestStrategy::None => VersionResult {
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
    ) -> VersionResult {
        let strategy = meta.and_then(|m| m.strategy.as_ref()).unwrap_or(&self.strategy);
        match strategy {
            IngestStrategy::Optimistic => {
                let base = meta.and_then(|m| m.version).unwrap_or(current_version);
                VersionResult {
                    version: base + 1,
                    changed: true,
                }
            },
            IngestStrategy::Explicit => {
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
            IngestStrategy::HashBased => {
                // Logic usually handled by caller comparing hashes
                 VersionResult {
                    version: current_version + 1, // Placeholder
                    changed: true,
                }
            },
            IngestStrategy::None => VersionResult {
                version: 0,
                changed: false,
            },
        }
    }
}
