use crate::algebra::{Weight, ZSet};
use crate::types::{make_key, raw_id, Sp00kyValue};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A base collection (table) in the store.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Collection {
    pub name: String,
    /// Z-set tracking record membership and weights.
    pub zset: ZSet,
    /// Actual record data, keyed by raw record ID (without table prefix).
    pub rows: HashMap<String, Sp00kyValue>,
}

impl Collection {
    pub fn new(name: String) -> Self {
        Self {
            name,
            zset: HashMap::new(),
            rows: HashMap::new(),
        }
    }

    /// Apply a mutation to this collection. Returns (zset_key, weight).
    pub fn apply_mutation(
        &mut self,
        op: Operation,
        id: &str,
        data: Sp00kyValue,
    ) -> (String, Weight) {
        let weight = op.weight();
        let normalized = raw_id(id);
        match op {
            Operation::Create | Operation::Update => {
                self.rows.insert(normalized.to_string(), data);
            }
            Operation::Delete => {
                self.rows.remove(normalized);
            }
        }

        let key = make_key(&self.name, id);
        if weight != 0 {
            let entry = self.zset.entry(key.clone()).or_insert(0);
            *entry += weight;
            if *entry == 0 {
                self.zset.remove(&key);
            }
        }
        (key, weight)
    }

    /// Look up a row by its raw ID.
    pub fn get_row(&self, id: &str) -> Option<&Sp00kyValue> {
        self.rows.get(raw_id(id))
    }

    /// Get the version of a record from its `_00_rv` field.
    pub fn get_record_version(&self, id: &str) -> Option<i64> {
        self.rows
            .get(raw_id(id))?
            .get("_00_rv")?
            .as_f64()
            .map(|n| n as i64)
    }
}

/// The store holds all base collections (tables).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Store {
    pub collections: HashMap<String, Collection>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ensure_collection(&mut self, name: &str) -> &mut Collection {
        self.collections
            .entry(name.to_string())
            .or_insert_with(|| Collection::new(name.to_string()))
    }

    pub fn get_collection(&self, name: &str) -> Option<&Collection> {
        self.collections.get(name)
    }

    /// Apply a Change to the store. Returns (zset_key, weight).
    pub fn apply_change(&mut self, change: &Change) -> (String, Weight) {
        let coll = self.ensure_collection(&change.table);
        coll.apply_mutation(change.op, &change.id, change.data.clone())
    }

    /// Get row data by zset key (format "table:id").
    pub fn get_row_by_key(&self, key: &str) -> Option<&Sp00kyValue> {
        let (table, id) = crate::types::parse_key(key)?;
        let coll = self.collections.get(table)?;
        // Try raw ID first, then with table prefix
        coll.rows.get(id).or_else(|| coll.rows.get(key))
    }

    /// Get the version of a record by its zset key (format "table:id").
    pub fn get_record_version_by_key(&self, key: &str) -> Option<i64> {
        let (table, id) = crate::types::parse_key(key)?;
        self.collections.get(table)?.get_record_version(id)
    }
}

/// A single record mutation.
#[derive(Debug, Clone)]
pub struct Change {
    pub table: String,
    pub op: Operation,
    pub id: String,
    pub data: Sp00kyValue,
}

impl Change {
    pub fn create(table: &str, id: &str, data: impl Into<Sp00kyValue>) -> Self {
        Self {
            table: table.to_string(),
            op: Operation::Create,
            id: id.to_string(),
            data: data.into(),
        }
    }

    pub fn update(table: &str, id: &str, data: impl Into<Sp00kyValue>) -> Self {
        Self {
            table: table.to_string(),
            op: Operation::Update,
            id: id.to_string(),
            data: data.into(),
        }
    }

    pub fn delete(table: &str, id: &str) -> Self {
        Self {
            table: table.to_string(),
            op: Operation::Delete,
            id: id.to_string(),
            data: Sp00kyValue::Null,
        }
    }
}

/// A batch of changes to apply in a single step.
#[derive(Debug, Clone, Default)]
pub struct ChangeSet {
    pub changes: Vec<Change>,
}

/// A record for initial bulk loading.
#[derive(Debug, Clone)]
pub struct Record {
    pub table: String,
    pub id: String,
    pub data: Sp00kyValue,
}

impl Record {
    pub fn new(table: &str, id: &str, data: impl Into<Sp00kyValue>) -> Self {
        Self {
            table: table.to_string(),
            id: id.to_string(),
            data: data.into(),
        }
    }
}

/// Mutation type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Create,
    Update,
    Delete,
}

impl Operation {
    pub fn weight(&self) -> Weight {
        match self {
            Operation::Create => 1,
            Operation::Update => 0,
            Operation::Delete => -1,
        }
    }

    pub fn changes_content(&self) -> bool {
        matches!(self, Operation::Create | Operation::Update)
    }

    /// Parse an operation from a string (case-insensitive).
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "CREATE" => Some(Operation::Create),
            "UPDATE" => Some(Operation::Update),
            "DELETE" => Some(Operation::Delete),
            _ => None,
        }
    }
}
