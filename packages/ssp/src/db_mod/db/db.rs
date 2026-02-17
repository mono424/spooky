use crate::spooky_record::SpookyReadable;
use crate::spooky_value::SpookyValue;
use crate::types::{FastMap, Operation, ZSet};
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

#[derive(Clone, Debug)]
pub struct Table {
    pub name: SmolStr,
    db: Arc<RedbDatabase>,
}

impl Table {
    pub fn new(name: SmolStr, db: Arc<RedbDatabase>) -> Self {
        Self { name, db }
    }

    pub fn get_record_version(&self, id: &str) -> Option<i64> {
        let raw_id = id.split_once(':').map(|(_, rest)| rest).unwrap_or(id);
        let read_txn = self.db.begin_read().ok()?;
        let table = read_txn.open_table(TABLE_VERSIONS).ok()?;
        let val = table.get(&(self.name.as_str(), raw_id)).ok()??;
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
        
        // Ensure we store with the raw ID (matching get_row_value logic)
        let (_, raw_id) = crate::types::parse_zset_key(&zset_key).expect("valid zset key");
        
        self.apply_mutation_impl(op, SmolStr::new(raw_id), data, weight, zset_key)
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
    pub fn get(&self, id: &str) -> Option<SpookyValue> {
        let raw_id = id.split_once(':').map(|(_, rest)| rest).unwrap_or(id);
        let read_txn = self.db.begin_read().ok()?;
        let table = read_txn.open_table(TABLE_RECORDS).ok()?;
        let val = table.get(&(self.name.as_str(), raw_id)).ok()??;
        let bytes = val.value();
        // Deserialize
        let (buf, count) = crate::serialization::from_bytes(bytes).ok()?;
        let record = crate::spooky_record::SpookyRecord::new(buf, count);
        Some(record.to_value()) 
    }

    pub fn get_zset_weight(&self, key: &str) -> i64 {
        let read_txn = self.db.begin_read().ok();
        if let Some(txn) = read_txn {
             if let Ok(table) = txn.open_table(TABLE_ZSET) {
                 // Try looking up the zset key directly - assuming caller passes full zset key (e.g. "table:id")
                 // The table key is (TableName, ZSetKey)
                 if let Ok(Some(val)) = table.get(&(self.name.as_str(), key)) {
                     return val.value();
                 }
             }
        }
        0
    }

    pub fn get_all_zset(&self) -> ZSet {
        let mut zset = FastMap::default();
        let read_txn = match self.db.begin_read() {
            Ok(txn) => txn,
            Err(_) => return zset,
        };
        let table = match read_txn.open_table(TABLE_ZSET) {
            Ok(t) => t,
            Err(_) => return zset,
        };
        
        // Iterate all keys in ZSet table
        // The key is (TableName, ZSetKey)
        // We only want keys matching self.name
        // Redb ranges support standard range syntax.
        // We need a range over (self.name, *)
        // Range on tuple key: start=(name, ""), end=(name, ~) ??
        // Redb tuple key ordering: lexicographical.
        // So (name, "") is start, (name + \0, "") is end?
        // Actually, we can just iterate and filter, or use range if possible.
        // Redb `range` takes a generic Range bounds.
        // For `(&str, &str)`, we can't easily construct a purely unbounded range on second component 
        // while bounding the first, unless we use the tuple ordering properties.
        // (name, "") .. (name, "\u{10FFFF}") should cover all strings for that name?
        
        let start = (self.name.as_str(), "");
        // Use a string strictly greater than any valid suffix. unicode max char?
        // Or just next string after name?
        // If we iterate all, it might be slow if many tables.
        // Let's try range with start (name, "") and end (name, bit pattern max).
        // Actually, let's just use `range(start..)` and break when table name changes.
        
        if let Ok(iter) = table.range(start..) {
            for res in iter {
                if let Ok((k, v)) = res {
                    let (t_name, key) = k.value();
                    if t_name != self.name.as_str() {
                        break;
                    }
                    zset.insert(SmolStr::new(key), v.value());
                }
            }
        }
        zset
    }

    pub fn len(&self) -> usize {
        let read_txn = match self.db.begin_read() {
            Ok(txn) => txn,
            Err(_) => return 0,
        };
        let table = match read_txn.open_table(TABLE_RECORDS) {
            Ok(t) => t,
            Err(_) => return 0,
        };
        
        // Iterate over range for this table name
        // (name, "") to (name, ~)
        // We use a safe upper bound logic or just iterate and break
        let start = (self.name.as_str(), "");
        let mut count = 0;
        if let Ok(iter) = table.range(start..) {
            for res in iter {
                if let Ok((k, _)) = res {
                    let (t_name, _) = k.value();
                    if t_name != self.name.as_str() {
                        break;
                    }
                    count += 1;
                }
            }
        }
        count
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains_key(&self, id: &str) -> bool {
        self.get(id).is_some()
    }
}

// ─── Database ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct Database {
    db: Arc<RedbDatabase>,
    // Stateless!
}

impl Database {
    pub fn ensure_table(&self, name: &str) -> Table {
        self.table(name)
    }

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
        })
    }


    pub fn apply_batch(
        &self,
        entries: Vec<(SmolStr, Operation, SmolStr, SpookyValue)>,
    ) -> Result<Vec<(SmolStr, SmolStr, i64, bool)>, redb::Error> {
        let write_txn = self.db.begin_write()?;
        let mut results = Vec::with_capacity(entries.len());
        {
            let mut records_table = write_txn.open_table(TABLE_RECORDS)?;
            let mut zset_table = write_txn.open_table(TABLE_ZSET)?;
            let mut versions_table = write_txn.open_table(TABLE_VERSIONS)?;

            for (table_name, op, key, data) in entries {
                let weight = op.weight();
                // We assume make_zset_key is available in crate::types as used elsewhere
                let zset_key = crate::types::make_zset_key(&table_name, &key);
                let content_changed = op.changes_content();
                
                // Ensure we store with the raw ID
                let (_, raw_id) = crate::types::parse_zset_key(&zset_key).expect("valid zset key");

                // 1. Records
                // Serialize data
                // We unwrap here because the signature doesn't support other errors easily, 
                // and serialization failure of internal types is critical/unexpected.
                // In production code we might want a custom error type wrapping redb::Error.
                match op {
                    Operation::Create | Operation::Update => {
                        let (buf, _) = crate::serialization::from_spooky(&data).expect("serialization failed");
                        records_table.insert(&(table_name.as_str(), raw_id), buf.as_slice())?;
                        
                        // Extract version if present
                        if let Some(v) = data.get("spooky_rv").and_then(|v| v.as_f64()) {
                            versions_table.insert(&(table_name.as_str(), raw_id), v as u64)?;
                        }
                    }
                    Operation::Delete => {
                        records_table.remove(&(table_name.as_str(), raw_id))?;
                        versions_table.remove(&(table_name.as_str(), raw_id))?;
                    }
                }


                // 2. ZSet
                if weight != 0 {
                    let current_weight = zset_table.get(&(table_name.as_str(), zset_key.as_str()))?
                        .map(|v| v.value())
                        .unwrap_or(0);
                    
                    let new_weight = current_weight + weight;
                    if new_weight == 0 {
                        zset_table.remove(&(table_name.as_str(), zset_key.as_str()))?;
                    } else {
                        zset_table.insert(&(table_name.as_str(), zset_key.as_str()), new_weight)?;
                    }
                }
                
                results.push((table_name, zset_key, weight, content_changed));
            }
        }
        write_txn.commit()?;
        Ok(results)
    }

    pub fn table(&self, name: &str) -> Table {
        Table::new(SmolStr::new(name), self.db.clone())
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
        let db = make_test_db();
        let table = db.table("users");
        assert_eq!(table.name, "users");
    }

    #[test]
    fn test_apply_mutation() {
        let db = make_test_db();
        let mut table = db.table("users");
        
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

        // Test Get
        let val = table.get("user:1").expect("should find record");
        // Simple check
        if let SpookyValue::Object(m) = val {
             assert_eq!(m.get("name").unwrap().as_str(), Some("Alice"));
        } else {
             panic!("expected object");
        }

        // Test ZSet retrieval
        let zset = table.get_all_zset();
        assert_eq!(zset.len(), 1);
        assert_eq!(zset.get(key.as_str()), Some(&1));
    }
}
