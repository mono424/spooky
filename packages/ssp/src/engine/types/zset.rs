use rustc_hash::FxHasher;
use smol_str::SmolStr;
use std::hash::BuildHasherDefault;

pub type Weight = i64;
pub type RowKey = SmolStr;
pub type FastMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FxHasher>>;
pub type ZSet = FastMap<RowKey, Weight>;
