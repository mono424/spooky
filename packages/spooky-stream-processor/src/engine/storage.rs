use crate::engine::view::{FastMap, RowKey, ZSet, SpookyValue};
use super::interner::{Symbol, SymbolTable};
use serde::{Deserialize, Serialize};

/// Columnar Storage Enum.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Column {
    Int(Vec<i64>),
    Float(Vec<f64>),
    Bool(Vec<bool>),
    Text(Vec<Symbol>),
}

impl Column {
    pub fn len(&self) -> usize {
        match self {
            Column::Int(v) => v.len(),
            Column::Float(v) => v.len(),
            Column::Bool(v) => v.len(),
            Column::Text(v) => v.len(),
        }
    }
    
    pub fn push_int(&mut self, v: i64) {
        if let Column::Int(vec) = self { vec.push(v); } else { panic!("Type mismatch: Expected Int"); }
    }
    
    pub fn push_float(&mut self, v: f64) {
        if let Column::Float(vec) = self { vec.push(v); } else { panic!("Type mismatch: Expected Float"); }
    }
    
    pub fn push_bool(&mut self, v: bool) {
        if let Column::Bool(vec) = self { vec.push(v); } else { panic!("Type mismatch: Expected Bool"); }
    }
    
    pub fn push_text(&mut self, v: Symbol) {
        if let Column::Text(vec) = self { vec.push(v); } else { panic!("Type mismatch: Expected Text"); }
    }
    
    // Efficient removal (swap_remove)
    pub fn swap_remove(&mut self, index: usize) {
         match self {
            Column::Int(v) => { v.swap_remove(index); },
            Column::Float(v) => { v.swap_remove(index); },
            Column::Bool(v) => { v.swap_remove(index); },
            Column::Text(v) => { v.swap_remove(index); },
        }
    }
}

/// A Columnar Table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Table {
    pub name: String,
    pub columns: FastMap<String, Column>,
    pub num_rows: usize,
    
    // Primary Key Index: Maps RowKey ("user:123") -> Row Index (0, 1, 2...)
    // This allows O(1) lookup of a row's data in the columns.
    #[serde(skip)]
    pub pk_map: FastMap<RowKey, usize>,
    
    // Inverted Index mapping Row Index -> RowKey (for iteration/rebuilds)
    #[serde(skip)]
    pub index_to_pk: Vec<RowKey>,

    // Diff/Delta tracking (Weights)
    pub zset: ZSet,
    pub hashes: FastMap<RowKey, String>,
}

impl Table {
    pub fn new(name: String) -> Self {
        Self {
            name,
            columns: FastMap::default(),
            num_rows: 0,
            pk_map: FastMap::default(),
            index_to_pk: Vec::new(),
            zset: FastMap::default(),
            hashes: FastMap::default(),
        }
    }
    
    pub fn apply_delta(&mut self, delta: &ZSet) {
        for (key, weight) in delta {
            let entry = self.zset.entry(key.clone()).or_insert(0);
            *entry += weight;
            if *entry == 0 {
                self.zset.remove(key);
            }
        }
    }

    /// Reconstructs a SpookyValue (Object) for a given Row Key.
    /// This is slow (re-allocation) but needed for legacy View logic compatibility.
    pub fn get_row_spooky(&self, key: &RowKey, interner: &SymbolTable) -> Option<SpookyValue> {
        let row_idx = *self.pk_map.get(key)?;
        
        let mut map = FastMap::default();
        
        for (col_name, col) in &self.columns {
            let val = match col {
                Column::Int(v) => SpookyValue::Number(v.get(row_idx).copied().unwrap_or(0) as f64),
                Column::Float(v) => SpookyValue::Number(v.get(row_idx).copied().unwrap_or(0.0)),
                Column::Bool(v) => SpookyValue::Bool(v.get(row_idx).copied().unwrap_or(false)),
                Column::Text(v) => {
                    let sym = v.get(row_idx).copied().unwrap_or(0);
                    // Resolve symbol
                    let s = interner.resolve(sym).unwrap_or_default();
                    SpookyValue::Str(smol_str::SmolStr::new(s))
                }
            };
            map.insert(smol_str::SmolStr::new(col_name), val);
        }
        
        Some(SpookyValue::Object(map))
    }
}
