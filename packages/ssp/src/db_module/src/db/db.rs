use crate::spooky_record::SpookyRecord;
use redb::{Database, ReadableDatabase, TableDefinition};
use std::path::Path;

// Table definitions
// Key: String (record ID/key)
// Value: Byte slice (serialized SpookyRecord)
const SCHEMA_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("schema");
const RECORDS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("records");
const VERSION_TABLE: TableDefinition<&str, u64> = TableDefinition::new("versions");
const ZSET_TABLE: TableDefinition<&str, i64> = TableDefinition::new("zset");

pub struct SpookyDb {
    db: Database,
}

impl SpookyDb {
    /// Open or create the database at the specified path.
    /// Also ensures that the required tables exist.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, redb::Error> {
        let db = Database::create(path)?;

        // Initialize tables within a write transaction
        let write_txn = db.begin_write()?;
        {
            let _ = write_txn.open_table(RECORDS_TABLE)?;
            let _ = write_txn.open_table(VERSION_TABLE)?;
            let _ = write_txn.open_table(ZSET_TABLE)?;
        }
        write_txn.commit()?;

        Ok(Self { db })
    }

    /// Ingest a record into the database.
    /// This functions as an upsert: if the key exists, it is overwritten.
    pub fn ingest_record(&self, key: &str, data: &[u8]) -> Result<(), redb::Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(RECORDS_TABLE)?;
            table.insert(key, data)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    /// Update an existing record.
    /// Currently synonymous with ingest_record (upsert).
    pub fn update_record(&self, key: &str, data: &[u8]) -> Result<(), redb::Error> {
        self.ingest_record(key, data)
    }

    /// Remove a record by its key.
    /// Returns true if the record existed and was removed, false otherwise.
    pub fn remove_record(&self, record_id: &str) -> Result<bool, redb::Error> {
        let write_txn = self.db.begin_write()?;
        let existed = {
            let mut table = write_txn.open_table(RECORDS_TABLE)?;
            table.remove(record_id)?.is_some()
        };
        write_txn.commit()?;
        Ok(existed)
    }

    /// Retrieve a record's raw bytes by key.
    pub fn get_record(&self, record_id: &str) -> Result<Option<Vec<u8>>, redb::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(RECORDS_TABLE)?;

        if let Some(access) = table.get(record_id)? {
            // We must copy the data out because the read transaction implementation
            // doesn't allow returning a reference that outlives the transaction easily in this simple API.
            // SpookyRecord is zero-copy but expects a slice. We return Vec<u8> here so caller can wrap it.
            let val = access.value();
            Ok(Some(val.to_vec()))
        } else {
            Ok(None)
        }
    }

    pub fn get_version(&self, record_id: &str) -> Result<Option<u64>, redb::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(VERSION_TABLE)?;
        Ok(table.get(record_id)?.map(|access| access.value()))
    }

    pub fn get_zset(&self, record_id: &str) -> Result<Option<i64>, redb::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(ZSET_TABLE)?;
        Ok(table.get(record_id)?.map(|access| access.value()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialization::from_cbor;
    use crate::spooky_record::record_mut::SpookyRecordMut;
    use tempfile::NamedTempFile;

    const BENCH_CBOR: &[u8] = &[
        172, 102, 97, 99, 116, 105, 118, 101, 245, 99, 97, 103, 101, 24, 28, 101, 99, 111, 117,
        110, 116, 25, 3, 232, 103, 100, 101, 108, 101, 116, 101, 100, 244, 103, 104, 105, 115, 116,
        111, 114, 121, 130, 162, 102, 97, 99, 116, 105, 111, 110, 101, 108, 111, 103, 105, 110,
        105, 116, 105, 109, 101, 115, 116, 97, 109, 112, 26, 73, 150, 2, 210, 162, 102, 97, 99,
        116, 105, 111, 110, 102, 117, 112, 100, 97, 116, 101, 105, 116, 105, 109, 101, 115, 116,
        97, 109, 112, 26, 73, 150, 2, 220, 98, 105, 100, 107, 117, 115, 101, 114, 58, 97, 98, 99,
        49, 50, 51, 104, 109, 101, 116, 97, 100, 97, 116, 97, 246, 107, 109, 105, 120, 101, 100,
        95, 97, 114, 114, 97, 121, 132, 24, 42, 100, 116, 101, 120, 116, 245, 161, 102, 110, 101,
        115, 116, 101, 100, 101, 118, 97, 108, 117, 101, 100, 110, 97, 109, 101, 101, 65, 108, 105,
        99, 101, 103, 112, 114, 111, 102, 105, 108, 101, 163, 102, 97, 118, 97, 116, 97, 114, 120,
        30, 104, 116, 116, 112, 115, 58, 47, 47, 101, 120, 97, 109, 112, 108, 101, 46, 99, 111,
        109, 47, 97, 118, 97, 116, 97, 114, 46, 106, 112, 103, 99, 98, 105, 111, 113, 83, 111, 102,
        116, 119, 97, 114, 101, 32, 101, 110, 103, 105, 110, 101, 101, 114, 104, 115, 101, 116,
        116, 105, 110, 103, 115, 163, 109, 110, 111, 116, 105, 102, 105, 99, 97, 116, 105, 111,
        110, 115, 245, 103, 112, 114, 105, 118, 97, 99, 121, 162, 101, 108, 101, 118, 101, 108, 3,
        102, 112, 117, 98, 108, 105, 99, 244, 101, 116, 104, 101, 109, 101, 100, 100, 97, 114, 107,
        101, 115, 99, 111, 114, 101, 251, 64, 88, 224, 0, 0, 0, 0, 0, 100, 116, 97, 103, 115, 131,
        105, 100, 101, 118, 101, 108, 111, 112, 101, 114, 100, 114, 117, 115, 116, 104, 100, 97,
        116, 97, 98, 97, 115, 101,
    ];

    #[test]
    fn test_spooky_db_basics() -> Result<(), Box<dyn std::error::Error>> {
        let tmp_file = NamedTempFile::new()?;
        let db_path = tmp_file.path();

        let db = SpookyDb::new(db_path)?;

        let cbor_data: cbor4ii::core::Value = cbor4ii::serde::from_slice(BENCH_CBOR)?;
        let (data, fields) = from_cbor(&cbor_data)?;
        let key = "user:123";

        // 1. Ingest
        db.ingest_record(key, &data)?;

        // 2. Get
        let fetched_data = db.get_record(key)?.expect("Record should exist");
        assert_eq!(fetched_data, data);

        // 3. Update (change value)
        let mut rec_mut = SpookyRecordMut::new(data.clone(), fields);
        rec_mut.set_i64("id", 456)?; // Change ID
        let updated_record = rec_mut.as_record();
        db.update_record(key, &updated_record.data_buf)?;

        let fetched_updated = db.get_record(key)?.expect("Record should exist");
        assert_eq!(fetched_updated, rec_mut.data_buf);

        // 4. Remove
        let removed = db.remove_record(key)?;
        assert!(removed);

        let fetched_after_remove = db.get_record(key)?;
        assert!(fetched_after_remove.is_none());

        Ok(())
    }

    /*
    #[test]
    fn test_badges_table_exists() -> Result<(), Box<dyn std::error::Error>> {
        let tmp_file = NamedTempFile::new()?;
        let db_path = tmp_file.path();

        let db = SpookyDb::new(db_path)?;

        // Verify we can write to badges table (even if we don't have specific methods for it yet)
        let write_txn = db.db.begin_write()?;
        {
            let mut table = write_txn.open_table(BADGES_TABLE)?;
            table.insert("badge:1", b"badge_data".as_slice())?;
        }
        write_txn.commit()?;

        let read_txn = db.db.begin_read()?;
        let table = read_txn.open_table(BADGES_TABLE)?;
        let val = table.get("badge:1")?;
        assert_eq!(val.unwrap().value(), b"badge_data");

        Ok(())
    }
    */
}
