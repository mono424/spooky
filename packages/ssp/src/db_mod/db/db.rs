use crate::spooky_record::SpookyRecord;
use crate::spooky_value::{FastMap, SpookyValue};
use crate::types::{Operation, ZSet};
use redb::{Database as RedbDatabase, ReadableDatabase, ReadableTable, TableDefinition};
use smol_str::SmolStr;
use std::path::Path;
use std::sync::Arc;

// ─── Constants ──────────────────────────────────────────────────────────────

// (TableName, RecordId) -> Data
const TABLE_RECORDS: TableDefinition<(&str, &str), &[u8]> = TableDefinition::new("v1_records");
// (TableName, ZSetKey) -> Weight
const TABLE_ZSET: TableDefinition<(&str, &str), i64> = TableDefinition::new("v1_zset");
// (TableName, RecordId) -> Version
const TABLE_VERSIONS: TableDefinition<(&str, &str), u64> = TableDefinition::new("v1_versions");

// ─── Table ──────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Table {
    pub name: SmolStr,
    db: Arc<RedbDatabase>,
}

impl Table {
    pub fn new(name: SmolStr, db: Arc<RedbDatabase>) -> Self {
        Self { name, db }
    }

    pub fn get_record_version(&self, id: &str) -> Option<i64> {
        let read_txn = self.db.begin_read().ok()?;
        let table = read_txn.open_table(TABLE_VERSIONS).ok()?;
        let val = table.get(&(self.name.as_str(), id)).ok()??;
        Some(val.value() as i64)
    }

    pub fn reserve(&mut self, _additional: usize) {
        // No-op for redb
    }

    pub fn apply_mutation(
        &mut self,
        op: Operation,
        key: SmolStr,
        data: SpookyValue,
    ) -> (SmolStr, i64) {
        let weight = op.weight();
        let zset_key = crate::types::make_zset_key(&self.name, &key);
        self.apply_mutation_impl(op, key, data, weight, zset_key)
    }

    fn apply_mutation_impl(
        &self,
        op: Operation,
        key: SmolStr,
        data: SpookyValue,
        weight: i64,
        zset_key: SmolStr,
    ) -> (SmolStr, i64) {
         let write_txn = self.db.begin_write().expect("failed to begin write txn");
         {
            // 1. Records
            // Using composite key (TableName, RecordId)
            let mut records = write_txn.open_table(TABLE_RECORDS).expect("open records");
            match op {
                Operation::Create | Operation::Update => {
                    let (buf, _) = crate::serialization::from_spooky(&data).expect("serialize");
                    records.insert(&(self.name.as_str(), key.as_str()), buf.as_slice()).expect("insert");
                    
                    if let Some(v) = data.get("spooky_rv").and_then(|v| v.as_f64()) {
                         let mut versions = write_txn.open_table(TABLE_VERSIONS).expect("open versions");
                         versions.insert(&(self.name.as_str(), key.as_str()), v as u64).expect("insert version");
                    }
                }
                Operation::Delete => {
                    records.remove(&(self.name.as_str(), key.as_str())).expect("remove");
                    let mut versions = write_txn.open_table(TABLE_VERSIONS).expect("open versions");
                    versions.remove(&(self.name.as_str(), key.as_str())).expect("remove version");
                }
            }

            // 2. ZSet
            if weight != 0 {
                let mut zset = write_txn.open_table(TABLE_ZSET).expect("open zset");
                let current_weight = zset.get(&(self.name.as_str(), zset_key.as_str()))
                    .expect("get zset")
                    .map(|v| v.value())
                    .unwrap_or(0);
                
                let new_weight = current_weight + weight;
                if new_weight == 0 {
                    zset.remove(&(self.name.as_str(), zset_key.as_str())).expect("remove zset");
                } else {
                    zset.insert(&(self.name.as_str(), zset_key.as_str()), new_weight).expect("insert zset");
                }
            }
         }
         write_txn.commit().expect("commit");
         (zset_key, weight)
    }

    pub fn apply_delta(&mut self, delta: &ZSet) {
        let write_txn = self.db.begin_write().expect("begin write");
        {
            let mut zset_table = write_txn.open_table(TABLE_ZSET).expect("open zset");
            for (key, weight) in delta {
                // key in delta is zset_key
                let current = zset_table.get(&(self.name.as_str(), key.as_str())).unwrap().map(|v| v.value()).unwrap_or(0);
                let new_val = current + weight;
                if new_val == 0 {
                    zset_table.remove(&(self.name.as_str(), key.as_str())).unwrap();
                } else {
                    zset_table.insert(&(self.name.as_str(), key.as_str()), new_val).unwrap();
                }
            }
        }
        write_txn.commit().unwrap();
    }
}

// ─── Database ───────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Database {
    db: Arc<RedbDatabase>,
    tables: FastMap<String, Table>,
}

impl Database {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, redb::Error> {
        let db = RedbDatabase::create(path)?;
        let db_arc = Arc::new(db);
        
        // Initialize tables
        let write_txn = db_arc.begin_write()?;
        {
            let _ = write_txn.open_table(TABLE_RECORDS)?;
            let _ = write_txn.open_table(TABLE_ZSET)?;
            let _ = write_txn.open_table(TABLE_VERSIONS)?;
        }
        write_txn.commit()?;

        Ok(Self {
            db: db_arc,
            tables: FastMap::default(),
        })
    }

    pub fn ensure_table(&mut self, name: &str) -> &mut Table {
        self.tables.entry(name.to_string()).or_insert_with(|| {
            Table::new(SmolStr::new(name), self.db.clone())
        })
    }

    pub fn get_table(&self, name: &str) -> Option<&Table> {
        self.tables.get(name)
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spooky_obj;
    use tempfile::NamedTempFile;

    fn make_test_db() -> Database {
        let tmp = NamedTempFile::new().unwrap();
        Database::new(tmp.path()).unwrap()
    }

    #[test]
    fn test_db_ensure_table() {
        let mut db = make_test_db();
        let table = db.ensure_table("users");
        assert_eq!(table.name, "users");
    }

    #[test]
    fn test_apply_mutation() {
        let mut db = make_test_db();
        let table = db.ensure_table("users");
        
        // spooky_obj! macro creates a SpookyValue::Object
        let data = spooky_obj!({ "name" => "Alice", "spooky_rv" => 1 });
        let (key, weight) = table.apply_mutation(
            Operation::Create,
            "user:1".into(),
            data
        );
        
        // ZSet key should be constructed
        assert_eq!(weight, 1);
        
        // Verify persistence
        let ver = table.get_record_version("user:1");
        assert_eq!(ver, Some(1));
    }
}
