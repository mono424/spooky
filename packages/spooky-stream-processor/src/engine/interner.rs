use std::sync::RwLock;
use crate::engine::view::FastMap;
use serde::{Serialize, Deserialize, Serializer, Deserializer};

pub type Symbol = u32;

/// A thread-safe String Interner.
/// Maps string contents to a unique u32 ID (Symbol).
/// This allows the engine to store integers instead of strings, saving memory and improving comparison speed.
#[derive(Debug)]
pub struct SymbolTable {
    map: RwLock<FastMap<String, Symbol>>,
    vec: RwLock<Vec<String>>,
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            map: RwLock::new(FastMap::default()),
            vec: RwLock::new(Vec::new()),
        }
    }

    /// Intern a string, returning its unique Symbol ID.
    /// If the string respects the implementation limits (e.g. u32::MAX), it is stored.
    pub fn get_or_intern(&self, val: &str) -> Symbol {
        // Fast path: read lock
        {
            let map = self.map.read().unwrap();
            if let Some(&id) = map.get(val) {
                return id;
            }
        }

        // Slow path: write lock
        let mut map = self.map.write().unwrap();
        let mut vec = self.vec.write().unwrap();

        // Check again in case another thread inserted it
        if let Some(&id) = map.get(val) {
            return id;
        }

        let id = vec.len() as Symbol;
        let s = val.to_string();
        vec.push(s.clone());
        map.insert(s, id);
        id
    }

    pub fn resolve(&self, id: Symbol) -> Option<String> {
        let vec = self.vec.read().unwrap();
        vec.get(id as usize).cloned() // Cloning here is safe but implies allocation on read. 
        // Returning &str is hard due to RwLockGuard lifetime.
    }
}

impl Serialize for SymbolTable {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Only serialize the vector. The map can be rebuilt.
        let vec = self.vec.read().unwrap();
        vec.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SymbolTable {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let vec: Vec<String> = Vec::deserialize(deserializer)?;
        let mut map = FastMap::default();
        for (i, s) in vec.iter().enumerate() {
            map.insert(s.clone(), i as u32);
        }
        
        Ok(SymbolTable {
            map: RwLock::new(map),
            vec: RwLock::new(vec),
        })
    }
}
