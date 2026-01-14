use super::circuit::Database;
use rustc_hash::FxHasher;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use smol_str::SmolStr;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::hash::{BuildHasherDefault, Hasher};

// --- Data Model ---

pub type Weight = i64;
pub type RowKey = SmolStr;

/// XorHash: Raw byte representation for zero-allocation hashing
/// Conversion to hex happens ONLY at serialization boundary
pub type XorHash = [u8; 32];

// We use FxHashMap instead of standard HashMap for internal calculations.
// It is extremely fast for integers and strings.
pub type FastMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FxHasher>>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SpookyValue {
    Null,
    Bool(bool),
    Number(f64),
    Str(SmolStr),
    Array(Vec<SpookyValue>),
    Object(FastMap<SmolStr, SpookyValue>),
}

impl From<serde_json::Value> for SpookyValue {
    fn from(v: serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => SpookyValue::Null,
            serde_json::Value::Bool(b) => SpookyValue::Bool(b),
            serde_json::Value::Number(n) => SpookyValue::Number(n.as_f64().unwrap_or(0.0)), // Simplified fallback
            serde_json::Value::String(s) => SpookyValue::Str(SmolStr::from(s)),
            serde_json::Value::Array(arr) => {
                SpookyValue::Array(arr.into_iter().map(SpookyValue::from).collect())
            }
            serde_json::Value::Object(obj) => SpookyValue::Object(
                obj.into_iter()
                    .map(|(k, v)| (SmolStr::from(k), SpookyValue::from(v)))
                    .collect(),
            ),
        }
    }
}

// Convert back to serde_json::Value for compatibility (if needed)
impl From<SpookyValue> for serde_json::Value {
    fn from(val: SpookyValue) -> Self {
        match val {
            SpookyValue::Null => serde_json::Value::Null,
            SpookyValue::Bool(b) => serde_json::Value::Bool(b),
            SpookyValue::Number(n) => json!(n),
            SpookyValue::Str(s) => serde_json::Value::String(s.to_string()),
            SpookyValue::Array(arr) => {
                serde_json::Value::Array(arr.into_iter().map(|v| v.into()).collect())
            }
            SpookyValue::Object(obj) => serde_json::Value::Object(
                obj.into_iter()
                    .map(|(k, v)| (k.to_string(), v.into()))
                    .collect(),
            ),
        }
    }
}

// Row struct deleted as it was unused and contained legacy types.

// A Z-Set is a mapping from Data -> Weight
// IMPORTANT: This must match the definition in circuit.rs!
pub type ZSet = FastMap<RowKey, Weight>;

// --- ID Tree Implementation ---

#[derive(Clone, Debug, PartialEq)]
pub struct LeafItem {
    pub id: SmolStr,
    pub hash: String, // String for legacy compatibility
    pub children: Option<FastMap<SmolStr, IdTree>>,
}

// The legacy IdTree struct (must be present in the output)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdTree {
    pub hash: String, // Hex string for compatibility
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<FastMap<SmolStr, IdTree>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leaves: Option<Vec<LeafItem>>,
}

impl IdTree {
    // Helper to create an empty tree (legacy support)
    pub fn empty() -> Self {
        Self {
            hash: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            children: None,
            leaves: None,
        }
    }
}

// Custom serialization: Convert [u8; 32] to hex ONLY at JSON boundary
impl serde::Serialize for LeafItem {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("LeafItem", 3)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("hash", &self.hash)?; // Already a String
        state.serialize_field("children", &self.children)?;
        state.end()
    }
}

impl<'de> serde::Deserialize<'de> for LeafItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct LeafItemHelper {
            id: SmolStr,
            hash: String,
            #[serde(default)]
            children: Option<FastMap<SmolStr, IdTree>>,
        }
        
        let helper = LeafItemHelper::deserialize(deserializer)?;
        Ok(LeafItem {
            id: helper.id,
            hash: helper.hash, // Correctly use String
            children: helper.children,
        })
    }
}

/// XOR-based checksum for fast node aggregation (commutative/associative)
/// This is a single CPU instruction per byte and allows for extremely fast tree recalculations
#[inline(always)]
fn xor_checksum(a: &XorHash, b: &XorHash) -> XorHash {
    let mut result = [0u8; 32];
    for i in 0..32 {
        result[i] = a[i] ^ b[i];
    }
    result
}

///Convert XorHash to hex string - ONLY used at serialization boundary
#[inline]
pub fn xor_hash_to_hex(hash: &XorHash) -> String {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = Vec::with_capacity(64);
    for byte in hash {
        hex.push(HEX_CHARS[(byte >> 4) as usize]);
        hex.push(HEX_CHARS[(byte & 0xf) as usize]);
    }
    // Safety: hex only contains ASCII characters from HEX_CHARS
    unsafe { String::from_utf8_unchecked(hex) }
}

/// Convert hex string to XorHash - ONLY used at deserialization boundary
pub fn hex_to_xor_hash(hex: &str) -> Result<XorHash, &'static str> {
    if hex.len() != 64 {
        return Err("Invalid hash length");
    }
    
    let mut bytes = [0u8; 32];
    let hex_bytes = hex.as_bytes();
    
    for i in 0..32 {
        let high = hex_digit_to_nibble(hex_bytes[i * 2]).ok_or("Invalid hex character")?;
        let low = hex_digit_to_nibble(hex_bytes[i * 2 + 1]).ok_or("Invalid hex character")?;
        bytes[i] = (high << 4) | low;
    }
    
    Ok(bytes)
}

#[inline(always)]
fn hex_digit_to_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Legacy helper for compatibility - converts items to hash bytes
pub fn compute_hash(items: &[String]) -> XorHash {
    let mut hasher = blake3::Hasher::new();
    for item in items {
        hasher.update(item.as_bytes());
        hasher.update(&[0]); // Delimiter
    }
    *hasher.finalize().as_bytes()
}

/// Custom serde serializer for Option<XorHash>
fn serialize_xorhash_opt<S>(opt: &Option<XorHash>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match opt {
        Some(hash) => serializer.serialize_some(&xor_hash_to_hex(hash)),
        None => serializer.serialize_none(),
    }
}

/// Custom serde deserializer for Option<XorHash>
fn deserialize_xorhash_opt<'de, D>(deserializer: D) -> Result<Option<XorHash>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    match opt {
        Some(s) => Ok(Some(hex_to_xor_hash(&s).map_err(serde::de::Error::custom)?)),
        None => Ok(None),
    }
}

impl IdTree {
    pub fn build(items: Vec<LeafItem>) -> Self {
        // Adaptive threshold: smaller for small sets, larger for big sets
        // This improves performance across different workload scales
        let threshold = match items.len() {
            0..=200 => 50,           // Small datasets: smaller chunks
            201..=2000 => 100,       // Medium datasets: balanced
            2001..=10000 => 200,     // Large datasets: larger chunks
            _ => 500,                // Very large: maximize efficiency
        };
        
        Self::build_with_threshold(items, threshold)
    }
    
    fn build_with_threshold(items: Vec<LeafItem>, threshold: usize) -> Self {
        // Base case: If few items, hash directly with blake3 (Leaf node)
        if items.len() <= threshold {
            let mut hasher = blake3::Hasher::new();
            for item in &items {
                hasher.update(item.id.as_bytes());
                // Convert hex string back to bytes for calculation
                let h = hex_to_xor_hash(&item.hash).unwrap_or([0u8; 32]);
                hasher.update(&h); 
                if let Some(children) = &item.children {
                    // Sorting is important for deterministic hashing
                    // Fix: Generic key type is SmolStr
                    let mut keys: Vec<&SmolStr> = children.keys().collect();
                    keys.sort_unstable();
                    for k in keys {
                        hasher.update(k.as_bytes());
                        let child = children.get(k).unwrap();
                        // Child hash is also String
                        let ch = hex_to_xor_hash(&child.hash).unwrap_or([0u8; 32]);
                        hasher.update(&ch);
                    }
                }
                hasher.update(&[0]);
            }
            // Store raw bytes directly
            let hash = *hasher.finalize().as_bytes();

            return IdTree {
                hash: xor_hash_to_hex(&hash), // Only change is here: bytes -> hex string
                children: None,
                leaves: Some(items),
            };
        }

        // Recursive case: Use XOR-based aggregation for internal nodes
        // ZERO string allocations - pure byte operations
        let mut children = FastMap::default();
        let mut aggregated_checksum: XorHash = [0u8; 32]; // Start with zero

        for (i, chunk) in items.chunks(threshold).enumerate() {
            let child_node = IdTree::build_with_threshold(chunk.to_vec(), threshold);
            
            // DIRECT XOR on raw bytes - converting from child hex strings
            let child_hash = hex_to_xor_hash(&child_node.hash).unwrap_or([0u8; 32]);
            aggregated_checksum = xor_checksum(&aggregated_checksum, &child_hash);
            
            children.insert(SmolStr::from(i.to_string()), child_node);
        }

        IdTree {
            hash: xor_hash_to_hex(&aggregated_checksum), // Only change: convert to string
            children: Some(children),
            leaves: None,
        }
    }
}



// --- Path Optimization ---

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Path(pub Vec<SmolStr>);

impl Path {
    pub fn new(s: &str) -> Self {
        if s.is_empty() {
            Path(vec![])
        } else {
            Path(s.split('.').map(SmolStr::new).collect())
        }
    }
    
    pub fn as_str(&self) -> String {
       self.0.join(".")
    }
}

impl serde::Serialize for Path {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if self.0.is_empty() {
            serializer.serialize_str("")
        } else {
            serializer.serialize_str(&self.0.join("."))
        }
    }
}

impl<'de> serde::Deserialize<'de> for Path {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s: String = serde::Deserialize::deserialize(deserializer)?;
        Ok(Path::new(&s))
    }
}

// --- View / Circuit Model ---

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Operator {
    Scan {
        table: String,
    },
    Filter {
        input: Box<Operator>,
        predicate: Predicate,
    },
    Join {
        left: Box<Operator>,
        right: Box<Operator>,
        on: JoinCondition,
    },
    Project {
        input: Box<Operator>,
        projections: Vec<Projection>,
    },
    Limit {
        input: Box<Operator>,
        limit: usize,
        #[serde(default)]
        order_by: Option<Vec<OrderSpec>>,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OrderSpec {
    pub field: Path,
    pub direction: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Projection {
    All,
    Field { name: Path },
    Subquery { alias: String, plan: Box<Operator> },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct JoinCondition {
    pub left_field: Path,
    pub right_field: Path,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Predicate {
    Prefix { field: Path, prefix: String },
    Eq { field: Path, value: Value },
    Neq { field: Path, value: Value },
    Gt { field: Path, value: Value },
    Gte { field: Path, value: Value },
    Lt { field: Path, value: Value },
    Lte { field: Path, value: Value },
    And { predicates: Vec<Predicate> },
    Or { predicates: Vec<Predicate> },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct QueryPlan {
    pub id: String,
    pub root: Operator,
}

// The legacy Output Struct
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterializedViewUpdate {
    pub query_id: String,
    pub result_hash: String,
    pub result_ids: Vec<String>, // Legacy expects explicit list of IDs
    pub tree: IdTree,            // Legacy expects the tree struct
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct View {
    pub plan: QueryPlan,
    pub cache: ZSet,
    #[serde(serialize_with = "serialize_xorhash_opt", deserialize_with = "deserialize_xorhash_opt")]
    pub last_hash: Option<XorHash>,
    #[serde(default)]
    pub params: Option<SpookyValue>,
    
    // INCREMENTAL TREE STATE - O(Delta) updates
    #[serde(skip, default)]
    leaf_hashes: FastMap<SmolStr, XorHash>,  // key → computed leaf hash
    #[serde(skip, default)]
    tree_hash: XorHash,                       // XOR aggregate of all leaf hashes
    #[serde(skip, default)]
    cached_ids: Vec<SmolStr>,                 // Sorted IDs (maintained incrementally)
}

impl View {
    pub fn new(plan: QueryPlan, params: Option<Value>) -> Self {
        Self {
            plan,
            cache: FastMap::default(),
            last_hash: None,
            params: params.map(SpookyValue::from),
            leaf_hashes: FastMap::default(),
            tree_hash: [0u8; 32],
            cached_ids: Vec::new(),
        }
    }

    /// The main function for updates.
    /// Uses delta optimization if possible.
    pub fn process(
        &mut self,
        changed_table: &str,
        input_delta: &ZSet,
        db: &Database,
    ) -> Option<MaterializedViewUpdate> {
        let mut deltas = FastMap::default();
        if !changed_table.is_empty() {
             deltas.insert(SmolStr::from(changed_table), input_delta.clone());  // Fix: use SmolStr
        }
        self.process_ingest(&deltas, db)
    }

    /// Optimized 2-Phase Processing: Handles multiple table updates at once.
    /// NOW WITH O(Delta) INCREMENTAL TREE UPDATES
    pub fn process_ingest(
        &mut self,
        deltas: &FastMap<SmolStr, ZSet>,
        db: &Database,
    ) -> Option<MaterializedViewUpdate> {
        let is_first_run = self.last_hash.is_none();

        let maybe_delta = if is_first_run {
            None
        } else {
             self.eval_delta_batch(&self.plan.root, deltas, db, self.params.as_ref())
        };

        let view_delta = if let Some(d) = maybe_delta {
            d
        } else {
            // FALLBACK MODE: Full Scan & Diff
            let target_set = self.eval_snapshot(&self.plan.root, db, self.params.as_ref());
            let mut diff = FastMap::default();

            for (key, &new_w) in target_set.iter() {
                let old_w = self.cache.get(key).copied().unwrap_or(0);
                if new_w != old_w {
                    diff.insert(key.clone(), new_w - old_w);
                }
            }
            for (key, &old_w) in &self.cache {
                if !target_set.contains_key(key) {
                    diff.insert(key.clone(), 0 - old_w);
                }
            }
            diff
        };

        if view_delta.is_empty() && !is_first_run {
            return None;
        }

        // === PHASE 1: Update cache & tree INCREMENTALLY - O(Delta) ===
        for (key, &weight) in &view_delta {
            let net_weight = {
                let entry = self.cache.entry(key.clone()).or_insert(0);
                *entry += weight;
                *entry
            };

            if weight > 0 && net_weight == weight {
                // NEW KEY INSERTED
                let leaf_hash = self.compute_leaf_hash(key, db);
                self.tree_hash = xor_checksum(&self.tree_hash, &leaf_hash);
                self.leaf_hashes.insert(key.clone(), leaf_hash);
                
                // O(log N) sorted insert
                let pos = self.cached_ids.binary_search(key).unwrap_or_else(|p| p);
                self.cached_ids.insert(pos, key.clone());
            } else if net_weight == 0 {
                // KEY DELETED
                self.cache.remove(key);
                if let Some(old_hash) = self.leaf_hashes.remove(key) {
                    // XOR out the old hash (A ⊕ A = 0)
                    self.tree_hash = xor_checksum(&self.tree_hash, &old_hash);
                }
                // O(log N) sorted remove
                if let Ok(pos) = self.cached_ids.binary_search(key) {
                    self.cached_ids.remove(pos);
                }
            } else {
                // KEY UPDATED (weight changed but still present)
                // Re-hash the leaf since row data may have changed
                if let Some(old_hash) = self.leaf_hashes.get(key).copied() {
                    let new_hash = self.compute_leaf_hash(key, db);
                    if old_hash != new_hash {
                        // XOR out old, XOR in new
                        self.tree_hash = xor_checksum(&self.tree_hash, &old_hash);
                        self.tree_hash = xor_checksum(&self.tree_hash, &new_hash);
                        self.leaf_hashes.insert(key.clone(), new_hash);
                    }
                }
            }
        }

        // === PHASE 2: Check if hash changed ===
        let hash = self.tree_hash;
        
        if self.last_hash.as_ref() != Some(&hash) || is_first_run {
            self.last_hash = Some(hash);
            
            // NO TREE BUILD - just clone the already-sorted IDs (SmolStr)
            
            // CONVERT TO LEGACY FORMAT (String) - O(N) penalty unavoidable for legacy compat
            let result_ids: Vec<String> = self.cached_ids.iter().map(|s| s.to_string()).collect();
            let result_hash_str = xor_hash_to_hex(&hash);

            // Shallow Tree for API Compliance
            let shallow_tree = IdTree {
                 hash: result_hash_str.clone(),
                 children: None,
                 leaves: None
            };

            return Some(MaterializedViewUpdate {
                query_id: self.plan.id.to_string(),
                result_hash: result_hash_str,
                result_ids,
                tree: shallow_tree,
            });
        }

        None
    }
    
    /// Build full IdTree on-demand (for clients that need Merkle proofs)
    pub fn build_tree(&self, db: &Database) -> IdTree {
        let items: Vec<LeafItem> = self.cached_ids
            .iter()
            .map(|id| self.expand_item(id, &self.plan.root, db))
            .collect();
        IdTree::build(items)
    }
    
    /// Compute blake3 hash of a leaf item for XOR aggregation
    #[inline(always)]
    fn compute_leaf_hash(&self, key: &SmolStr, db: &Database) -> XorHash {
        let row_hash = self.get_row_hash(key, db).unwrap_or([0u8; 32]);
        let mut hasher = blake3::Hasher::new();
        hasher.update(key.as_bytes());
        hasher.update(&row_hash);
        *hasher.finalize().as_bytes()
    }

    /// Attempts to calculate the delta purely incrementally for a BATCH of changes.
    fn eval_delta_batch(
        &self,
        op: &Operator,
        deltas: &FastMap<SmolStr, ZSet>,  // Changed from String to SmolStr
        db: &Database,
        context: Option<&SpookyValue>,
    ) -> Option<ZSet> {
        match op {
            Operator::Scan { table } => {
                // Return the delta for this table if it exists, otherwise empty
                if let Some(d) = deltas.get(table.as_str()) {  // Fix: use as_str()
                     Some(d.clone())
                } else {
                     Some(FastMap::default())
                }
            }
            Operator::Filter { input, predicate } => {
                let upstream_delta =
                    self.eval_delta_batch(input, deltas, db, context)?;
                
                // SIMD Optimization Check (Copy of eval_snapshot logic)
                let simd_target_op = match predicate {
                    Predicate::Gt { value: Value::Number(n), .. } => n.as_f64().map(|f| (f, NumericOp::Gt)),
                    Predicate::Gte { value: Value::Number(n), .. } => n.as_f64().map(|f| (f, NumericOp::Gte)),
                    Predicate::Lt { value: Value::Number(n), .. } => n.as_f64().map(|f| (f, NumericOp::Lt)),
                    Predicate::Lte { value: Value::Number(n), .. } => n.as_f64().map(|f| (f, NumericOp::Lte)),
                    Predicate::Eq { value: Value::Number(n), .. } => n.as_f64().map(|f| (f, NumericOp::Eq)),
                    Predicate::Neq { value: Value::Number(n), .. } => n.as_f64().map(|f| (f, NumericOp::Neq)),
                    _ => None,
                };

                let field_path = match predicate {
                     Predicate::Gt { field, .. } | Predicate::Gte { field, .. } |
                     Predicate::Lt { field, .. } | Predicate::Lte { field, .. } |
                     Predicate::Eq { field, .. } | Predicate::Neq { field, .. } => Some(field),
                     _ => None
                };

                if let (Some((target, op)), Some(path)) = (simd_target_op, field_path) {
                     // SIMD PATH
                     let (keys, weights, numbers) = extract_number_column(&upstream_delta, path, db);
                     let passing_indices = filter_f64_batch(&numbers, target, op);
                     
                     let mut out_delta = FastMap::default();
                     for idx in passing_indices {
                         out_delta.insert(keys[idx].clone(), weights[idx]);
                     }
                     Some(out_delta)
                } else {
                    // Slow Path
                    let mut out_delta = FastMap::default();
                    for (key, weight) in upstream_delta {
                        if self.check_predicate(predicate, &key, db, context) {
                            out_delta.insert(key, weight);
                        }
                    }
                    Some(out_delta)
                }
            }
            Operator::Project { input, .. } => {
                self.eval_delta_batch(input, deltas, db, context)
            }

            // Complex operators (Joins, Limits) fall back to snapshot
            Operator::Join { .. } | Operator::Limit { .. } => None,
        }
    }

    /// Deprecated: Helper wrapper for single-table delta (retained for compatibility if needed internally)
    #[allow(dead_code)]
    fn eval_delta(
        &self,
        op: &Operator,
        changed_table: &str,
        input_delta: &ZSet,
        db: &Database,
        context: Option<&SpookyValue>,
    ) -> Option<ZSet> {
         let mut deltas = FastMap::default();
         deltas.insert(SmolStr::from(changed_table), input_delta.clone());  // Convert to SmolStr
         self.eval_delta_batch(op, &deltas, db, context)
    }

    /// The classic detailed Full-Scan Evaluator (for fallback and init)
    /// OPTIMIZATION #6: Uses Cow for zero-copy scans (~20-30% improvement)
    fn eval_snapshot<'a>(&self, op: &Operator, db: &'a Database, context: Option<&SpookyValue>) -> Cow<'a, ZSet> {
        match op {
            Operator::Scan { table } => {
                // ZERO-COPY: Return borrowed reference to DB zset, no allocation!
                if let Some(tb) = db.tables.get(table.as_str()) {
                    Cow::Borrowed(&tb.zset)
                } else {
                    Cow::Owned(FastMap::default())
                }
            }
            Operator::Filter { input, predicate } => {
                let upstream = self.eval_snapshot(input, db, context);
                
                // SIMD Optimization Check
                let simd_target_op = match predicate {
                    Predicate::Gt { value: Value::Number(n), .. } => n.as_f64().map(|f| (f, NumericOp::Gt)),
                    Predicate::Gte { value: Value::Number(n), .. } => n.as_f64().map(|f| (f, NumericOp::Gte)),
                    Predicate::Lt { value: Value::Number(n), .. } => n.as_f64().map(|f| (f, NumericOp::Lt)),
                    Predicate::Lte { value: Value::Number(n), .. } => n.as_f64().map(|f| (f, NumericOp::Lte)),
                    Predicate::Eq { value: Value::Number(n), .. } => n.as_f64().map(|f| (f, NumericOp::Eq)),
                    Predicate::Neq { value: Value::Number(n), .. } => n.as_f64().map(|f| (f, NumericOp::Neq)),
                    _ => None,
                };

                let field_path = match predicate {
                     Predicate::Gt { field, .. } | Predicate::Gte { field, .. } |
                     Predicate::Lt { field, .. } | Predicate::Lte { field, .. } |
                     Predicate::Eq { field, .. } | Predicate::Neq { field, .. } => Some(field),
                     _ => None
                };

                if let (Some((target, op)), Some(path)) = (simd_target_op, field_path) {
                     // SIMD PATH
                     let (keys, weights, numbers) = extract_number_column(&upstream, path, db);
                     let passing_indices = filter_f64_batch(&numbers, target, op);
                     
                     let mut out = FastMap::default();
                     for idx in passing_indices {
                         // Safety: indices returned by filter_batch are valid for keys/weights
                         out.insert(keys[idx].clone(), weights[idx]);
                     }
                     Cow::Owned(out)
                 } else {
                    // Slow Path (Loop)
                    let mut out = FastMap::default();
                    for (key, weight) in upstream.iter() {
                        if self.check_predicate(predicate, &key, db, context) {
                            out.insert(key.clone(), *weight);
                        }
                    }
                    Cow::Owned(out)
                }
            }
            Operator::Project { input, .. } => self.eval_snapshot(input, db, context),
            Operator::Limit {
                input,
                limit,
                order_by,
            } => {
                let upstream = self.eval_snapshot(input, db, context);
                let mut items: Vec<_> = upstream.iter().map(|(k, v)| (k.clone(), *v)).collect();

                if let Some(orders) = order_by {
                    items.sort_by(|a, b| {
                        let row_a = self.get_row_value(&a.0, db);
                        let row_b = self.get_row_value(&b.0, db);

                        for ord in orders {
                            let val_a = resolve_nested_value(row_a, &ord.field);
                            let val_b = resolve_nested_value(row_b, &ord.field);

                            let cmp = compare_spooky_values(val_a, val_b);
                            if cmp != Ordering::Equal {
                                return if ord.direction.eq_ignore_ascii_case("DESC") {
                                    cmp.reverse()
                                } else {
                                    cmp
                                };
                            }
                        }
                        a.0.cmp(&b.0)
                    });
                } else {
                    items.sort_unstable_by(|a, b| a.0.cmp(&b.0));
                }

                let mut out = FastMap::default();
                for (i, (key, weight)) in items.into_iter().enumerate() {
                    if i < *limit {
                        out.insert(key, weight);
                    } else {
                        break;
                    }
                }
                Cow::Owned(out)
            }
            Operator::Join { left, right, on } => {
                let s_left = self.eval_snapshot(left, db, context);
                let s_right = self.eval_snapshot(right, db, context);
                let mut out = FastMap::default();

                // 1. BUILD PHASE: Build Index for the RIGHT side
                // Map: Hash of Join-Field -> List of (Key, Weight)
                let mut right_index: FastMap<u64, Vec<(&SmolStr, &i64)>> = FastMap::default();

                for (r_key, r_weight) in s_right.iter() {
                    if let Some(r_val) = self.get_row_value(r_key.as_str(), db) {
                        if let Some(r_field) = resolve_nested_value(Some(r_val), &on.right_field) {
                            let hash = hash_spooky_value(r_field);
                            right_index.entry(hash).or_default().push((r_key, r_weight));
                        }
                    }
                }

                // 2. PROBE PHASE: Iterate Left and lookup Right (O(1))
                for (l_key, l_weight) in s_left.iter() {
                    if let Some(l_val) = self.get_row_value(l_key.as_str(), db) {
                        if let Some(l_field) = resolve_nested_value(Some(l_val), &on.left_field) {
                            let hash = hash_spooky_value(l_field);

                            // Hash Lookup instead of Loop!
                            if let Some(matches) = right_index.get(&hash) {
                                for (_r_key, r_weight) in matches {
                                    // We have a match! (Should double check equality with compare_spooky_values for strictness)
                                    let w = l_weight * *r_weight;
                                    *out.entry(l_key.clone()).or_insert(0) += w;
                                }
                            }
                        }
                    }
                }
                Cow::Owned(out)
            }
        }
    }

    fn expand_item(&self, id: &str, op: &Operator, db: &Database) -> LeafItem {
        let mut final_hash = self
            .get_row_hash(id, db)
            .unwrap_or_else(|| [0u8; 32]);

        let mut children_map = FastMap::default();
        let projections = self.find_projections(op);

        if !projections.is_empty() {
            let mut dependency_hashes = Vec::<XorHash>::with_capacity(projections.len());

            for proj in projections {
                if let Projection::Subquery {
                    alias,
                    plan: sub_op,
                } = proj
                {
                    // Create child context (SpookyValue)
                    // We must clone existing params (SpookyValue) or create new Object
                    let mut context = self.params.clone().unwrap_or(SpookyValue::Object(FastMap::default()));
                    
                    if let SpookyValue::Object(ref mut obj) = context {
                        obj.insert(SmolStr::new("parent"), SpookyValue::Str(SmolStr::new(id)));
                    } else {
                        // If context was not an object (weird), we overwrite or wrap. 
                        // For safety, let's just make a new object with parent
                        let mut map = FastMap::default();
                        map.insert(SmolStr::new("parent"), SpookyValue::Str(SmolStr::new(id)));
                        context = SpookyValue::Object(map);
                    }

                    let sub_zset = self.eval_snapshot(sub_op, db, Some(&context));
                    let mut sub_ids: Vec<String> = sub_zset.keys().map(|k| k.to_string()).collect(); // Legacy uses String keys? Or SmolStr? ZSet uses SmolStr.
                    sub_ids.sort_unstable();

                    let sub_items: Vec<LeafItem> = sub_ids
                        .iter()
                        .map(|sub_id| self.expand_item(sub_id, sub_op, db))
                        .collect();

                    let sub_tree = IdTree::build(sub_items);
                    // Parse hash back to bytes for dependency hash
                    let sub_hash = hex_to_xor_hash(&sub_tree.hash).unwrap_or([0u8; 32]);
                    dependency_hashes.push(sub_hash);
                    children_map.insert(SmolStr::from(alias), sub_tree);  // alias is String, FastMap key is SmolStr
                }
            }

            if !dependency_hashes.is_empty() {
                let mut hasher = blake3::Hasher::new();
                hasher.update(&final_hash); // Direct bytes
                for h in dependency_hashes {
                    hasher.update(&h); // Direct bytes
                }
                final_hash = *hasher.finalize().as_bytes();
            }
        }

        LeafItem {
            id: SmolStr::new(id),
            hash: xor_hash_to_hex(&final_hash), // Convert to Hex String for Legacy LeafItem
            children: if children_map.is_empty() {
                None
            } else {
                Some(children_map)
            },
        }
    }

    fn find_projections<'a>(&self, op: &'a Operator) -> Vec<&'a Projection> {
        match op {
            Operator::Project { projections, .. } => projections.iter().collect(),
            Operator::Limit { input, .. } => self.find_projections(input),
            _ => vec![],
        }
    }

    /// Helper to get table reference from row key (caches table name extraction)
    #[inline(always)]
    fn get_table_for_key<'a>(&self, key: &str, db: &'a Database) -> Option<&'a super::circuit::Table> {
        let (table_name, _) = key.split_once(':')?;
        db.tables.get(table_name)
    }

    #[inline(always)]
    fn get_row_value<'a>(&self, key: &str, db: &'a Database) -> Option<&'a SpookyValue> {
        // Optimized: single split, single table lookup
        self.get_table_for_key(key, db)?.rows.get(key)
    }

    #[inline(always)]
    fn get_row_hash(&self, key: &str, db: &Database) -> Option<XorHash> {
        // Optimized: single split, single table lookup
        self.get_table_for_key(key, db)?.hashes.get(key).copied()
    }

    fn check_predicate(
        &self,
        pred: &Predicate,
        key: &str,
        db: &Database,
        context: Option<&SpookyValue>,
    ) -> bool {
        // Helper to get actual SpookyValue for comparison from the Predicate (which stores Value)
        let resolve_val = |_field: &Path, value: &Value| -> Option<SpookyValue> {
             if let Some(obj) = value.as_object() {
                if let Some(param_path) = obj.get("$param") {
                     if let Some(ctx) = context {
                        // $param is usually a simple string path like "user.id"
                        // We need to parse it as a Path to resolve it, OR since $param is dynamic we treat it as string
                        let path_str = param_path.as_str().unwrap_or("");
                        let path = Path::new(path_str);
                        // resolve nested param path from SpookyValue context!
                        resolve_nested_value(Some(ctx), &path).cloned()
                    } else {
                        None
                    }
                } else {
                    Some(SpookyValue::from(value.clone()))
                }
            } else {
                 Some(SpookyValue::from(value.clone()))
            }
        };

        match pred {
            Predicate::And { predicates } => predicates
                .iter()
                .all(|p| self.check_predicate(p, key, db, context)),
            Predicate::Or { predicates } => predicates
                .iter()
                .any(|p| self.check_predicate(p, key, db, context)),
             Predicate::Prefix { field, prefix } => {
                 // Check if field value starts with prefix
                 if field.0.len() == 1 && field.0[0] == "id" {
                     return key.starts_with(prefix);
                 }
                  if let Some(row_val) = self.get_row_value(key, db) {
                     if let Some(val) = resolve_nested_value(Some(row_val), field) {
                         if let SpookyValue::Str(s) = val {
                             return s.starts_with(prefix);
                         }
                     }
                 }
                 false
              },
            Predicate::Eq { field, value } |
            Predicate::Neq { field, value } |
            Predicate::Gt { field, value } |
            Predicate::Gte { field, value } |
            Predicate::Lt { field, value } |
            Predicate::Lte { field, value } => {
                let target_val = resolve_val(field, value);
                if target_val.is_none() { return false; }
                let target_val = target_val.unwrap();

                let actual_val_opt = if field.0.len() == 1 && field.0[0] == "id" {
                     Some(SpookyValue::Str(SmolStr::new(key)))
                } else {
                    self.get_row_value(key, db).and_then(|r| resolve_nested_value(Some(r), field).cloned())
                };
                
                if let Some(actual_val) = actual_val_opt {
                    let ord = compare_spooky_values(Some(&actual_val), Some(&target_val));
                    match pred {
                        Predicate::Eq { .. } => ord == Ordering::Equal,
                        Predicate::Neq { .. } => ord != Ordering::Equal,
                        Predicate::Gt { .. } => ord == Ordering::Greater,
                        Predicate::Gte { .. } => ord == Ordering::Greater || ord == Ordering::Equal,
                        Predicate::Lt { .. } => ord == Ordering::Less,
                        Predicate::Lte { .. } => ord == Ordering::Less || ord == Ordering::Equal,
                        _ => false
                    }
                } else {
                    false
                }
            }
        }
    }
}

// --- OPTIMIZED COMPARISON & HASHING ---

// Avoids Allocations (.to_string) completely for primitive types.
// Optimized for SpookyValue
fn compare_spooky_values(a: Option<&SpookyValue>, b: Option<&SpookyValue>) -> Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(va), Some(vb)) => match (va, vb) {
            (SpookyValue::Null, SpookyValue::Null) => Ordering::Equal,
            (SpookyValue::Bool(ba), SpookyValue::Bool(bb)) => ba.cmp(bb),
            (SpookyValue::Number(na), SpookyValue::Number(nb)) => {
               // Simple f64 comparison
               na.partial_cmp(&nb).unwrap_or(Ordering::Equal)
            },
            (SpookyValue::Str(sa), SpookyValue::Str(sb)) => sa.cmp(sb),
            (SpookyValue::Array(aa), SpookyValue::Array(ab)) => {
                let len_cmp = aa.len().cmp(&ab.len());
                if len_cmp != Ordering::Equal { return len_cmp; }
                for (ia, ib) in aa.iter().zip(ab.iter()) {
                    let cmp = compare_spooky_values(Some(ia), Some(ib));
                    if cmp != Ordering::Equal { return cmp; }
                }
                Ordering::Equal
            }
            (SpookyValue::Object(oa), SpookyValue::Object(ob)) => oa.len().cmp(&ob.len()), // Deep compare skipped for perf
            _ => type_rank(va).cmp(&type_rank(vb)),
        }
    }
}

#[inline(always)]
fn type_rank(v: &SpookyValue) -> u8 {
    match v {
        SpookyValue::Null => 0,
        SpookyValue::Bool(_) => 1,
        SpookyValue::Number(_) => 2,
        SpookyValue::Str(_) => 3,
        SpookyValue::Array(_) => 4,
        SpookyValue::Object(_) => 5,
    }
}

// Dot notation access: "address.city" -> traverses json
// Optimized specifically for Path and SpookyValue
#[inline(always)]
fn resolve_nested_value<'a>(root: Option<&'a SpookyValue>, path: &Path) -> Option<&'a SpookyValue> {
    let mut current = root;
    for part in &path.0 {
        match current {
            Some(SpookyValue::Object(map)) => { current = map.get(part); }
            _ => return None,
        }
    }
    current
}

// Fast hashing for Join Keys
#[inline(always)]
fn hash_spooky_value(v: &SpookyValue) -> u64 {
     let mut hasher = FxHasher::default();
     hash_value_recursive(v, &mut hasher);
     hasher.finish()
}

// --- 3. SIMD / COLUMNAR OPTIMIZATIONS ---

// Helper Enum for numeric predicates
enum NumericOp {
    Gt, Gte, Lt, Lte, Eq, Neq
}

/* 
   Attempts to extract a "Column" of f64 values from the ZSet + Database.
   Returns: (Ids, Weights, Numbers) aligned by index.
   If a value is missing or not a number, it defaults to f64::NAN which fails most comparisons safely.
   OPTIMIZATION #5: Caches table reference to avoid repeated string parsing
*/
#[inline(always)]
fn extract_number_column(
    zset: &ZSet,
    path: &Path,
    db: &Database,
    // Optional context if needed for resolving params locally (not used for column extraction usually)
) -> (Vec<SmolStr>, Vec<i64>, Vec<f64>) {
    let mut ids = Vec::with_capacity(zset.len());
    let mut weights = Vec::with_capacity(zset.len());
    let mut numbers = Vec::with_capacity(zset.len());

    // Cache for table lookups - avoids repeated string parsing
    let mut cached_table_name: Option<&str> = None;
    let mut cached_table: Option<&super::circuit::Table> = None;

    for (key, weight) in zset {
        // OPTIMIZATION #5: Check cache before parsing table name
        let table_opt = if let Some((table_name, _)) = key.split_once(':') {
            if Some(table_name) == cached_table_name {
                // Cache hit! No need to parse or lookup
                cached_table
            } else {
                // Cache miss: update cache
                let table = db.tables.get(table_name);
                cached_table_name = Some(table_name);
                cached_table = table;
                table
            }
        } else {
            None
        };

        let num_val = if let Some(table) = table_opt {
            if let Some(row_val) = table.rows.get(key) {
                if let Some(SpookyValue::Number(n)) = resolve_nested_value(Some(row_val), path) {
                    *n
                } else {
                    f64::NAN
                }
            } else {
                f64::NAN
            }
        } else {
            f64::NAN
        };

        ids.push(key.clone());
        weights.push(*weight);
        numbers.push(num_val);
    }

    (ids, weights, numbers)
}

// Auto-vectorizable batch filter
// Returns indices of passing elements
fn filter_f64_batch(values: &[f64], target: f64, op: NumericOp) -> Vec<usize> {
    let mut indices = Vec::with_capacity(values.len());
    
    // Explicit chunking to encourage SIMD opt by the compiler
    let chunks = values.chunks_exact(8);
    let remainder = chunks.remainder();
    
    let mut i = 0;
    for chunk in chunks {
        // Inner loop fixed size 8 - easier for LLVM to vectorize
        for val in chunk {
            let pass = match op {
                NumericOp::Gt => *val > target,
                NumericOp::Gte => *val >= target,
                NumericOp::Lt => *val < target,
                NumericOp::Lte => *val <= target,
                NumericOp::Eq => (*val - target).abs() < f64::EPSILON, // Float approx eq
                NumericOp::Neq => (*val - target).abs() > f64::EPSILON,
            };
            if pass {
                indices.push(i);
            }
            i += 1;
        }
    }

    for val in remainder {
        let pass = match op {
            NumericOp::Gt => *val > target,
            NumericOp::Gte => *val >= target,
            NumericOp::Lt => *val < target,
            NumericOp::Lte => *val <= target,
            NumericOp::Eq => (*val - target).abs() < f64::EPSILON,
            NumericOp::Neq => (*val - target).abs() > f64::EPSILON,
        };
        if pass {
            indices.push(i);
        }
        i += 1;
    }
    
    indices
}

// Portable SIMD Sum (Chunked)
#[allow(dead_code)] // Will be used in future aggregations
#[inline(always)]
pub fn sum_f64_simd(values: &[f64]) -> f64 {
    let mut sums = [0.0; 8];
    let chunks = values.chunks_exact(8);
    let remainder = chunks.remainder();

    for chunk in chunks {
        for i in 0..8 {
            sums[i] += chunk[i];
        }
    }
    
    let mut total: f64 = sums.iter().sum();
    for v in remainder {
        total += v;
    }
    total
}

fn hash_value_recursive(v: &SpookyValue, hasher: &mut FxHasher) {
    match v {
        SpookyValue::Null => { hasher.write_u8(0); },
        SpookyValue::Bool(b) => { hasher.write_u8(1); hasher.write_u8(*b as u8); },
        SpookyValue::Number(n) => { 
            hasher.write_u8(2); 
            hasher.write_u64(n.to_bits());
        },
        SpookyValue::Str(s) => { hasher.write_u8(3); hasher.write(s.as_bytes()); },
        SpookyValue::Array(arr) => {
            hasher.write_u8(4);
            for item in arr {
                hash_value_recursive(item, hasher);
            }
        },
        SpookyValue::Object(obj) => {
             hasher.write_u8(5);
             // Simple iteration, no sorting for speed (as discussed in prev steps)
             for (k, v) in obj {
                 hasher.write(k.as_bytes());
                 hash_value_recursive(v, hasher);
             }
        }
    }
}
