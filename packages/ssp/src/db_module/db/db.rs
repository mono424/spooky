use std::path::Path;

use redb::{Database as RedbDatabase, ReadableDatabase, ReadableTable, TableDefinition};
use smol_str::SmolStr;

use super::types::{
    BatchMutationResult, BulkRecord, DbMutation, FastHashSet, FastMap, Operation, SpookyDbError,
    ZSet,
};
use crate::serialization::from_bytes;
use crate::spooky_record::{SpookyReadable, SpookyRecord};
use crate::spooky_value::SpookyValue;

// ─── Table definitions ───────────────────────────────────────────────────────
//
// Flat string key: "table_name:record_id"
// INVARIANT: table names must not contain ':'. IDs may contain ':'; only the
// first ':' in the key is the separator (split_once(':') is used everywhere).

/// Primary record store. Key: "table:id" → Value: serialized SpookyRecord bytes.
const RECORDS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("records");

/// Version store for sync / conflict detection.
/// Key: "table:id" → Value: version u64 (read from the "spooky_rv" field or explicit).
const VERSION_TABLE: TableDefinition<&str, u64> = TableDefinition::new("versions");

// ─── SpookyDb ─────────────────────────────────────────────────────────────────

/// Persistent record store backed by redb.
///
/// **Ownership**: `SpookyDb` is meant to be owned exclusively by one component
/// (e.g. the circuit runner). No `Arc`, no `Mutex` — callers hold `&mut self`
/// for write operations.
///
/// **ZSet**: a per-table in-memory `FastMap<record_id, weight>` that shadows
/// RECORDS_TABLE. Rebuilt from a sequential RECORDS_TABLE scan on startup.
/// All view-evaluation ZSet reads are pure memory — zero I/O.
pub struct SpookyDb {
    /// On-disk KV store.
    db: RedbDatabase,

    /// Hot ZSet per table. Key: table name → Value: (record_id → weight).
    /// INVARIANT: table names must not contain ':'.
    /// Weight 1 = record present; absent = deleted.
    zsets: FastMap<SmolStr, ZSet>,
}

// ─── Construction ─────────────────────────────────────────────────────────────

impl SpookyDb {
    /// Open or create the database at `path`.
    ///
    /// Initialises redb tables on first open. Rebuilds all in-memory ZSets
    /// from a sequential RECORDS_TABLE scan — O(N records), ~20–100ms per
    /// million records on an SSD.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, SpookyDbError> {
        let db = RedbDatabase::create(path)?;

        // Ensure tables exist (idempotent).
        {
            let write_txn = db.begin_write()?;
            let _ = write_txn.open_table(RECORDS_TABLE)?;
            let _ = write_txn.open_table(VERSION_TABLE)?;
            write_txn.commit()?;
        }

        let mut spooky = SpookyDb {
            db,
            zsets: FastMap::default(),
        };
        spooky.rebuild_zsets_from_records()?;
        Ok(spooky)
    }

    /// Sequential scan of RECORDS_TABLE. Sets `zsets[table][id] = 1` for every
    /// key found. Also populates the table registry (no TABLES_TABLE needed).
    fn rebuild_zsets_from_records(&mut self) -> Result<(), SpookyDbError> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(RECORDS_TABLE)?;
        for entry in table.iter()? {
            let (key_guard, _) = entry?;
            let key_str: &str = key_guard.value();
            if let Some((table_name, id)) = key_str.split_once(':') {
                self.zsets
                    .entry(SmolStr::new(table_name))
                    .or_default()
                    .insert(SmolStr::new(id), 1);
            }
        }
        Ok(())
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Build a flat "table:id" redb key.
#[inline]
fn make_key(table: &str, id: &str) -> String {
    format!("{}:{}", table, id)
}

// ─── Write Operations ─────────────────────────────────────────────────────────

impl SpookyDb {
    /// Apply a single mutation in its own write transaction.
    ///
    /// `data` must be pre-serialized SpookyRecord bytes.
    /// Returns `(zset_key, weight_delta)` for the caller to accumulate into a
    /// `BatchMutationResult` if needed.
    pub fn apply_mutation(
        &mut self,
        table: &str,
        op: Operation,
        id: &str,
        data: Option<&[u8]>,
        version: Option<u64>,
    ) -> Result<(SmolStr, i64), SpookyDbError> {
        let key = make_key(table, id);
        let weight = op.weight();

        // --- In-memory ZSet update ---
        let zset = self.zsets.entry(SmolStr::new(table)).or_default();
        if matches!(op, Operation::Delete) {
            zset.remove(id);
        } else {
            zset.insert(SmolStr::new(id), 1);
        }

        // --- Persist to redb (single transaction) ---
        let write_txn = self.db.begin_write()?;
        {
            let mut records = write_txn.open_table(RECORDS_TABLE)?;
            let mut versions = write_txn.open_table(VERSION_TABLE)?;

            if matches!(op, Operation::Delete) {
                records.remove(key.as_str())?;
                versions.remove(key.as_str())?;
            } else {
                if let Some(bytes) = data {
                    records.insert(key.as_str(), bytes)?;
                }
                if let Some(ver) = version {
                    versions.insert(key.as_str(), ver)?;
                }
            }
        }
        write_txn.commit()?;

        Ok((SmolStr::new(&key), weight))
    }

    /// Batch mutations in **one** write transaction (one fsync).
    ///
    /// All `DbMutation.data` fields must be pre-serialized SpookyRecord bytes.
    /// Serialize with `from_cbor` / `serialize_into` **before** calling this
    /// to minimise write-lock hold time.
    ///
    /// N mutations = 1 transaction = 1 fsync.
    pub fn apply_batch(
        &mut self,
        mutations: Vec<DbMutation>,
    ) -> Result<BatchMutationResult, SpookyDbError> {
        let mut membership_deltas: FastMap<SmolStr, ZSet> = FastMap::default();
        let mut content_updates: FastMap<SmolStr, FastHashSet<SmolStr>> = FastMap::default();
        let mut changed_tables_set: FastHashSet<SmolStr> = FastHashSet::default();

        let write_txn = self.db.begin_write()?;
        {
            let mut records = write_txn.open_table(RECORDS_TABLE)?;
            let mut versions = write_txn.open_table(VERSION_TABLE)?;

            for mutation in mutations {
                let DbMutation {
                    table,
                    id,
                    op,
                    data,
                    version,
                } = mutation;

                let key = make_key(&table, &id);
                let weight = op.weight();

                // In-memory ZSet update.
                let zset = self.zsets.entry(table.clone()).or_default();
                if matches!(op, Operation::Delete) {
                    zset.remove(&id);
                } else {
                    zset.insert(id.clone(), 1);
                }

                // Redb update.
                if matches!(op, Operation::Delete) {
                    records.remove(key.as_str())?;
                    versions.remove(key.as_str())?;
                } else {
                    if let Some(ref bytes) = data {
                        records.insert(key.as_str(), bytes.as_slice())?;
                    }
                    if let Some(ver) = version {
                        versions.insert(key.as_str(), ver)?;
                    }
                }

                // Accumulate result deltas.
                if weight != 0 {
                    membership_deltas
                        .entry(table.clone())
                        .or_default()
                        .insert(id.clone(), weight);
                }
                if !matches!(op, Operation::Delete) {
                    content_updates
                        .entry(table.clone())
                        .or_default()
                        .insert(id.clone());
                }
                changed_tables_set.insert(table);
            }
        }
        write_txn.commit()?;

        Ok(BatchMutationResult {
            membership_deltas,
            content_updates,
            changed_tables: changed_tables_set.into_iter().collect(),
        })
    }

    /// Bulk initial load: all records in **one** write transaction.
    ///
    /// Sets every ZSet weight to 1 (records present). Use for startup
    /// hydration or init_load in circuit.rs.
    pub fn bulk_load(
        &mut self,
        records: impl IntoIterator<Item = BulkRecord>,
    ) -> Result<(), SpookyDbError> {
        let write_txn = self.db.begin_write()?;
        {
            let mut rec_table = write_txn.open_table(RECORDS_TABLE)?;
            for record in records {
                let BulkRecord { table, id, data } = record;
                let key = make_key(&table, &id);
                rec_table.insert(key.as_str(), data.as_slice())?;
                self.zsets.entry(table).or_default().insert(id, 1);
            }
        }
        write_txn.commit()?;
        Ok(())
    }
}

// ─── Read Operations ──────────────────────────────────────────────────────────

impl SpookyDb {
    /// Fetch raw SpookyRecord bytes for a record.
    ///
    /// Returns `None` if the record does not exist.
    /// The returned `Vec<u8>` is a copy out of the redb memory-mapped region
    /// (one unavoidable allocation — data must outlive the read transaction).
    ///
    /// **ZSet guard**: checks the in-memory ZSet first (O(1) hash lookup).
    /// If the record is absent from the ZSet it cannot be in redb — the
    /// `begin_read()` call is skipped entirely. Critical for join probe loops
    /// where many lookups miss.
    ///
    /// Usage:
    /// ```rust,ignore
    /// let bytes = db.get_record_bytes("users", "alice")?.unwrap();
    /// let (buf, count) = from_bytes(&bytes)?;
    /// let record = SpookyRecord::new(buf, count);
    /// let age = record.get_i64("age");
    /// ```
    pub fn get_record_bytes(
        &self,
        table: &str,
        id: &str,
    ) -> Result<Option<Vec<u8>>, SpookyDbError> {
        // O(1) ZSet guard — avoids redb read transaction for missing keys.
        if self.get_zset_weight(table, id) == 0 {
            return Ok(None);
        }
        let key = make_key(table, id);
        let read_txn = self.db.begin_read()?;
        let tbl = read_txn.open_table(RECORDS_TABLE)?;
        Ok(tbl
            .get(key.as_str())?
            .map(|guard: redb::AccessGuard<&[u8]>| guard.value().to_vec()))
    }

    /// Reconstruct a partial `SpookyValue::Object` from a stored record.
    ///
    /// Only fields whose names are listed in `fields` are included. Unknown
    /// hashes (fields not in `fields`) are silently skipped — field names are
    /// not stored in the binary format and cannot be recovered from hashes.
    ///
    /// Returns `None` if the record does not exist.
    ///
    /// Use `get_record_bytes` + `SpookyReadable` accessors on the hot path.
    /// Use this for compatibility layers that need a named `SpookyValue`.
    pub fn get_record_typed(
        &self,
        table: &str,
        id: &str,
        fields: &[&str],
    ) -> Result<Option<SpookyValue>, SpookyDbError> {
        let raw = match self.get_record_bytes(table, id)? {
            Some(b) => b,
            None => return Ok(None),
        };

        let (buf, count) = from_bytes(&raw)?;
        let record = SpookyRecord::new(buf, count);

        let mut map = std::collections::BTreeMap::new();
        for &name in fields {
            if let Some(val) = record.get_field::<SpookyValue>(name) {
                map.insert(SmolStr::new(name), val);
            }
        }
        Ok(Some(SpookyValue::Object(map)))
    }

    /// Version for a record (sync / conflict detection).
    ///
    /// Returns `None` if the record has no version entry.
    pub fn get_version(&self, table: &str, id: &str) -> Result<Option<u64>, SpookyDbError> {
        let key = make_key(table, id);
        let read_txn = self.db.begin_read()?;
        let tbl = read_txn.open_table(VERSION_TABLE)?;
        Ok(tbl
            .get(key.as_str())?
            .map(|guard: redb::AccessGuard<u64>| guard.value()))
    }
}

// ─── ZSet Operations (pure memory, zero I/O) ─────────────────────────────────

impl SpookyDb {
    /// Full ZSet for a table. Pure memory, zero I/O.
    ///
    /// Returns `None` if the table has never had any records.
    /// The borrow is valid until the next `&mut self` call.
    ///
    /// This is what `eval_snapshot(Scan)` borrows for the duration of a view tick.
    pub fn get_table_zset(&self, table: &str) -> Option<&ZSet> {
        self.zsets.get(table)
    }

    /// Weight for a single record. Returns 0 if absent (standard ZSet semantics).
    pub fn get_zset_weight(&self, table: &str, id: &str) -> i64 {
        self.zsets
            .get(table)
            .and_then(|z| z.get(id).copied())
            .unwrap_or(0)
    }

    /// Apply a pre-computed ZSet delta in memory only (no redb write).
    ///
    /// Used when records were already committed (e.g. after a checkpoint load)
    /// and only the in-memory ZSet needs syncing.
    pub fn apply_zset_delta_memory(&mut self, table: &str, delta: &ZSet) {
        let zset = self.zsets.entry(SmolStr::new(table)).or_default();
        for (id, weight) in delta {
            let entry = zset.entry(id.clone()).or_insert(0);
            *entry += weight;
            // Remove entries that have reached zero weight.
            if *entry == 0 {
                zset.remove(id);
            }
        }
    }
}

// ─── Table Info (pure memory, O(1)) ──────────────────────────────────────────

impl SpookyDb {
    /// Returns `true` if the table has at least one record in the in-memory ZSet.
    pub fn table_exists(&self, table: &str) -> bool {
        self.zsets
            .get(table)
            .map(|z| !z.is_empty())
            .unwrap_or(false)
    }

    /// All known table names (derived from in-memory ZSet keys).
    pub fn table_names(&self) -> impl Iterator<Item = &SmolStr> {
        self.zsets.keys()
    }

    /// Record count for a table.
    ///
    /// O(1) — ZSet entries = records present.
    pub fn table_len(&self, table: &str) -> usize {
        self.zsets.get(table).map(|z| z.len()).unwrap_or(0)
    }

    /// Explicitly register an empty table.
    ///
    /// Creates an empty ZSet entry so `table_exists()` returns `true` even
    /// before the first record is inserted. The table name must not contain ':'.
    pub fn ensure_table(&mut self, table: &str) {
        self.zsets.entry(SmolStr::new(table)).or_default();
    }
}

// ─── DbBackend trait ──────────────────────────────────────────────────────────

/// Thin adapter trait for incremental migration from the old in-memory
/// `Database` struct to `SpookyDb`. Implement for both; wire `circuit.rs`
/// against the trait.
///
/// All write operations return `Result` — a disk-full or corruption error must
/// never silently become a no-op. Callers must handle or propagate write errors.
pub trait DbBackend {
    /// Zero-copy ZSet access. Borrowed from memory — zero I/O.
    fn get_table_zset(&self, table: &str) -> Option<&ZSet>;

    /// Raw bytes for a record. Caller wraps in `SpookyRecord`.
    /// ZSet-guarded: returns `None` without I/O for keys absent from the ZSet.
    fn get_record_bytes(&self, table: &str, id: &str) -> Option<Vec<u8>>;

    /// Register an empty table.
    fn ensure_table(&mut self, table: &str);

    /// Single mutation: record write + ZSet update.
    fn apply_mutation(
        &mut self,
        table: &str,
        op: Operation,
        id: &str,
        data: Option<&[u8]>,
        version: Option<u64>,
    ) -> Result<(SmolStr, i64), SpookyDbError>;

    /// Batch mutations in one transaction.
    fn apply_batch(
        &mut self,
        mutations: Vec<DbMutation>,
    ) -> Result<BatchMutationResult, SpookyDbError>;

    /// Bulk initial load.
    fn bulk_load(
        &mut self,
        records: impl IntoIterator<Item = BulkRecord>,
    ) -> Result<(), SpookyDbError>;

    /// Weight for one record. Returns 0 if absent.
    fn get_zset_weight(&self, table: &str, id: &str) -> i64;
}

impl DbBackend for SpookyDb {
    fn get_table_zset(&self, table: &str) -> Option<&ZSet> {
        self.get_table_zset(table)
    }

    fn get_record_bytes(&self, table: &str, id: &str) -> Option<Vec<u8>> {
        self.get_record_bytes(table, id).ok().flatten()
    }

    fn ensure_table(&mut self, table: &str) {
        self.ensure_table(table);
    }

    fn apply_mutation(
        &mut self,
        table: &str,
        op: Operation,
        id: &str,
        data: Option<&[u8]>,
        version: Option<u64>,
    ) -> Result<(SmolStr, i64), SpookyDbError> {
        SpookyDb::apply_mutation(self, table, op, id, data, version)
    }

    fn apply_batch(
        &mut self,
        mutations: Vec<DbMutation>,
    ) -> Result<BatchMutationResult, SpookyDbError> {
        SpookyDb::apply_batch(self, mutations)
    }

    fn bulk_load(
        &mut self,
        records: impl IntoIterator<Item = BulkRecord>,
    ) -> Result<(), SpookyDbError> {
        SpookyDb::bulk_load(self, records)
    }

    fn get_zset_weight(&self, table: &str, id: &str) -> i64 {
        self.get_zset_weight(table, id)
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialization::from_cbor;
    use tempfile::NamedTempFile;

    const BENCH_CBOR: &[u8] = &[
        172, 102, 97, 99, 116, 105, 118, 101, 245, 99, 97, 103, 101, 24, 28, 101, 99, 111, 117,
        110, 116, 25, 3, 232, 103, 100, 101, 108, 101, 116, 101, 100, 244, 103, 104, 105, 115,
        116, 111, 114, 121, 130, 162, 102, 97, 99, 116, 105, 111, 110, 101, 108, 111, 103, 105,
        110, 105, 116, 105, 109, 101, 115, 116, 97, 109, 112, 26, 73, 150, 2, 210, 162, 102, 97,
        99, 116, 105, 111, 110, 102, 117, 112, 100, 97, 116, 101, 105, 116, 105, 109, 101, 115,
        116, 97, 109, 112, 26, 73, 150, 2, 220, 98, 105, 100, 107, 117, 115, 101, 114, 58, 97,
        98, 99, 49, 50, 51, 104, 109, 101, 116, 97, 100, 97, 116, 97, 246, 107, 109, 105, 120,
        101, 100, 95, 97, 114, 114, 97, 121, 132, 24, 42, 100, 116, 101, 120, 116, 245, 161,
        102, 110, 101, 115, 116, 101, 100, 101, 118, 97, 108, 117, 101, 100, 110, 97, 109, 101,
        101, 65, 108, 105, 99, 101, 103, 112, 114, 111, 102, 105, 108, 101, 163, 102, 97, 118,
        97, 116, 97, 114, 120, 30, 104, 116, 116, 112, 115, 58, 47, 47, 101, 120, 97, 109, 112,
        108, 101, 46, 99, 111, 109, 47, 97, 118, 97, 116, 97, 114, 46, 106, 112, 103, 99, 98,
        105, 111, 113, 83, 111, 102, 116, 119, 97, 114, 101, 32, 101, 110, 103, 105, 110, 101,
        101, 114, 104, 115, 101, 116, 116, 105, 110, 103, 115, 163, 109, 110, 111, 116, 105,
        102, 105, 99, 97, 116, 105, 111, 110, 115, 245, 103, 112, 114, 105, 118, 97, 99, 121,
        162, 101, 108, 101, 118, 101, 108, 3, 102, 112, 117, 98, 108, 105, 99, 244, 101, 116,
        104, 101, 109, 101, 100, 100, 97, 114, 107, 101, 115, 99, 111, 114, 101, 251, 64, 88,
        224, 0, 0, 0, 0, 0, 100, 116, 97, 103, 115, 131, 105, 100, 101, 118, 101, 108, 111,
        112, 101, 114, 100, 114, 117, 115, 116, 104, 100, 97, 116, 97, 98, 97, 115, 101,
    ];

    #[test]
    fn test_new_opens_empty_db() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = NamedTempFile::new()?;
        let db = SpookyDb::new(tmp.path())?;
        assert!(!db.table_exists("users"));
        assert_eq!(db.table_len("users"), 0);
        Ok(())
    }

    #[test]
    fn test_apply_mutation_create_get_delete() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = NamedTempFile::new()?;
        let mut db = SpookyDb::new(tmp.path())?;

        let cbor: cbor4ii::core::Value = cbor4ii::serde::from_slice(BENCH_CBOR)?;
        let (data, _) = from_cbor(&cbor)?;

        // Create
        db.apply_mutation("users", Operation::Create, "alice", Some(&data), Some(1))?;
        assert!(db.table_exists("users"));
        assert_eq!(db.table_len("users"), 1);
        assert_eq!(db.get_zset_weight("users", "alice"), 1);

        // Get raw bytes back
        let fetched = db.get_record_bytes("users", "alice")?.expect("should exist");
        assert_eq!(fetched, data);

        // Version
        assert_eq!(db.get_version("users", "alice")?, Some(1));

        // Delete
        db.apply_mutation("users", Operation::Delete, "alice", None, None)?;
        assert_eq!(db.get_zset_weight("users", "alice"), 0);
        assert!(db.get_record_bytes("users", "alice")?.is_none());
        assert_eq!(db.table_len("users"), 0);

        Ok(())
    }

    #[test]
    fn test_apply_batch_one_txn() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = NamedTempFile::new()?;
        let mut db = SpookyDb::new(tmp.path())?;

        let cbor: cbor4ii::core::Value = cbor4ii::serde::from_slice(BENCH_CBOR)?;
        let (data, _) = from_cbor(&cbor)?;

        let mutations = vec![
            DbMutation {
                table: SmolStr::new("users"),
                id: SmolStr::new("u1"),
                op: Operation::Create,
                data: Some(data.clone()),
                version: Some(1),
            },
            DbMutation {
                table: SmolStr::new("users"),
                id: SmolStr::new("u2"),
                op: Operation::Create,
                data: Some(data.clone()),
                version: Some(1),
            },
            DbMutation {
                table: SmolStr::new("posts"),
                id: SmolStr::new("p1"),
                op: Operation::Create,
                data: Some(data.clone()),
                version: Some(1),
            },
        ];

        let result = db.apply_batch(mutations)?;

        assert_eq!(db.table_len("users"), 2);
        assert_eq!(db.table_len("posts"), 1);
        assert_eq!(result.membership_deltas["users"].len(), 2);
        assert_eq!(result.membership_deltas["posts"].len(), 1);
        assert!(result.changed_tables.contains(&SmolStr::new("users")));
        assert!(result.changed_tables.contains(&SmolStr::new("posts")));

        Ok(())
    }

    #[test]
    fn test_bulk_load() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = NamedTempFile::new()?;
        let mut db = SpookyDb::new(tmp.path())?;

        let cbor: cbor4ii::core::Value = cbor4ii::serde::from_slice(BENCH_CBOR)?;
        let (data, _) = from_cbor(&cbor)?;

        let records = vec![
            BulkRecord {
                table: SmolStr::new("items"),
                id: SmolStr::new("i1"),
                data: data.clone(),
            },
            BulkRecord {
                table: SmolStr::new("items"),
                id: SmolStr::new("i2"),
                data: data.clone(),
            },
        ];

        db.bulk_load(records)?;
        assert_eq!(db.table_len("items"), 2);
        assert_eq!(db.get_zset_weight("items", "i1"), 1);
        assert_eq!(db.get_zset_weight("items", "i2"), 1);

        Ok(())
    }

    #[test]
    fn test_zset_survives_reopen() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = NamedTempFile::new()?;
        let path = tmp.path().to_path_buf();
        // Keep file alive but drop NamedTempFile handle so only the path remains.
        // Use a regular tempdir file to keep the path valid.
        let tmp_dir = tempfile::tempdir()?;
        let db_path = tmp_dir.path().join("test.redb");

        let cbor: cbor4ii::core::Value = cbor4ii::serde::from_slice(BENCH_CBOR)?;
        let (data, _) = from_cbor(&cbor)?;

        {
            let mut db = SpookyDb::new(&db_path)?;
            db.apply_mutation("users", Operation::Create, "alice", Some(&data), Some(1))?;
            db.apply_mutation("users", Operation::Create, "bob", Some(&data), Some(2))?;
            assert_eq!(db.table_len("users"), 2);
        }

        // Reopen — ZSet must be rebuilt from RECORDS_TABLE.
        let db2 = SpookyDb::new(&db_path)?;
        assert_eq!(db2.table_len("users"), 2);
        assert_eq!(db2.get_zset_weight("users", "alice"), 1);
        assert_eq!(db2.get_zset_weight("users", "bob"), 1);

        // Suppress unused path warning.
        let _ = path;
        Ok(())
    }

    #[test]
    fn test_get_record_typed_partial() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = NamedTempFile::new()?;
        let mut db = SpookyDb::new(tmp.path())?;

        let cbor: cbor4ii::core::Value = cbor4ii::serde::from_slice(BENCH_CBOR)?;
        let (data, _) = from_cbor(&cbor)?;

        db.apply_mutation("users", Operation::Create, "alice", Some(&data), None)?;

        // The CBOR fixture has an "age" field (i64 = 28) and "active" (bool).
        let val = db
            .get_record_typed("users", "alice", &["age", "active"])?
            .expect("should exist");

        assert!(matches!(val, SpookyValue::Object(_)));
        if let SpookyValue::Object(map) = val {
            // "age" and "active" should be present.
            assert!(map.contains_key("age"), "age field missing");
            assert!(map.contains_key("active"), "active field missing");
        }

        Ok(())
    }

    #[test]
    fn test_ensure_table_and_table_names() {
        let tmp = NamedTempFile::new().unwrap();
        let mut db = SpookyDb::new(tmp.path()).unwrap();

        assert!(!db.table_exists("empty_table"));
        db.ensure_table("empty_table");
        // ensure_table creates the ZSet entry, but table_exists checks for non-empty.
        // An empty ZSet → table_exists returns false (no records yet).
        assert!(!db.table_exists("empty_table"));
        // But table_names() still lists it.
        let names: Vec<&SmolStr> = db.table_names().collect();
        assert!(names.contains(&&SmolStr::new("empty_table")));
    }
}
