# SSP Circuit Optimization Analysis v2

## Executive Summary

This document analyzes optimization strategies for the Spooky Stream Processor (SSP), including the planned migration to a persistent storage layer using **redb** (embedded key-value store) with optional **rkyv** (zero-copy deserialization).

| Optimization | Effort | Impact | Priority | With Persistent Storage |
|-------------|--------|--------|----------|------------------------|
| Parallel Views | Low ✅ | High 🚀 | **DO NOW** | Unchanged - storage is shared |
| Batch Ingestion | Low ✅ | Medium 📈 | **DO NOW** | Transactions become critical |
| View Caching | Medium | Medium 📈 | **Later** | Cache invalidation simpler |
| Circuit Sharding | High ⚠️ | High 🚀 | **Only if needed** | **Easier** - shared storage solves duplication |
| Message Queue | High ⚠️ | Medium 📈 | **Probably never** | Unchanged |
| **Persistent Storage** | **Medium** | **High 🚀** | **DO SOON** | Foundation for everything else |

---

## 6. Persistent Storage Layer (NEW)

### Why Move Away from In-Memory

Current architecture:
```rust
pub struct Circuit {
    pub db: Database,  // ← In-memory HashMap<String, Table>
    pub views: Vec<View>,
    pub dependency_list: FastMap<TableName, DependencyList>,
}
```

**Problems:**
1. Memory grows unbounded with data
2. Multi-circuit scenarios duplicate all records
3. Crash = total data loss (relies on SurrealDB for recovery)
4. Large datasets don't fit in memory

**Solution:** Extract storage to persistent key-value store (redb) shared across circuits.

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                         Application                              │
├─────────────────────────────────────────────────────────────────┤
│  Circuit A          Circuit B          Circuit C                │
│  (realtime)         (analytics)        (tenant X)               │
│  ┌─────────┐        ┌─────────┐        ┌─────────┐             │
│  │ Views   │        │ Views   │        │ Views   │             │
│  │ ZSets   │        │ ZSets   │        │ ZSets   │             │
│  └────┬────┘        └────┬────┘        └────┬────┘             │
│       │                  │                  │                   │
│       └──────────────────┼──────────────────┘                   │
│                          ▼                                      │
│  ┌───────────────────────────────────────────────────────────┐ │
│  │              StorageLayer (Arc<Storage>)                   │ │
│  │  ┌─────────────────────────────────────────────────────┐  │ │
│  │  │                    redb Database                     │  │ │
│  │  │  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌───────────┐  │  │ │
│  │  │  │ records │ │ fields  │ │  zsets  │ │view_cache │  │  │ │
│  │  │  │  table  │ │  table  │ │  table  │ │   table   │  │  │ │
│  │  │  └─────────┘ └─────────┘ └─────────┘ └───────────┘  │  │ │
│  │  └─────────────────────────────────────────────────────┘  │ │
│  └───────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
```

### Table Design

#### Option A: Whole-Record Storage (Simpler)

```
┌─────────────────────────────────────────────────────────────────┐
│ Table: records                                                   │
│ Key: "{table}:{id}" (e.g., "users:user_abc123")                 │
│ Value: Serialized SpookyValue (JSON or rkyv bytes)              │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│ Table: zsets                                                     │
│ Key: "{table}:{id}" (same as records)                           │
│ Value: i64 weight                                                │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│ Table: view_cache                                                │
│ Key: "{view_id}"                                                 │
│ Value: Serialized Vec<SpookyValue> + hash                       │
└─────────────────────────────────────────────────────────────────┘
```

**Pros:**
- Simple implementation
- Single read per record
- Easy to reason about

**Cons:**
- Must read entire record even for single field access
- Large records = wasted I/O

#### Option B: Field-Level Storage (Optimized for Partial Reads)

```
┌─────────────────────────────────────────────────────────────────┐
│ Table: record_meta                                               │
│ Key: "{table}:{id}"                                             │
│ Value: { version: i64, field_count: u32, schema_hash: u64 }     │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│ Table: record_fields                                             │
│ Key: "{table}:{id}:{field_name}"                                │
│     e.g., "users:user_abc123:email"                             │
│ Value: Serialized field value (typed or raw bytes)              │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│ Table: zsets                                                     │
│ Key: "{table}:{id}"                                             │
│ Value: i64 weight                                                │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│ Table: field_index (optional - for filtered queries)            │
│ Key: "{table}:{field}:{value_hash}:{id}"                        │
│ Value: () or original value                                      │
└─────────────────────────────────────────────────────────────────┘
```

**Pros:**
- Read only needed fields
- Partial updates don't rewrite whole record
- Enables field-level indexes

**Cons:**
- More complex implementation
- Multiple reads to reconstruct full record
- Key space explosion (N records × M fields)

#### Option C: Hybrid (Recommended) — Detailed Explanation

The hybrid approach combines the simplicity of whole-record storage (Option A) with the field-level access benefits of field-level storage (Option B). The key insight is: **store records as single values, but with an internal structure that allows partial reads**.

##### Core Concept: Self-Describing Record Format

Instead of storing raw JSON or requiring the full record to be deserialized, we store records with a **field offset index** at the beginning. This allows jumping directly to any field's bytes without parsing the entire record.

```
┌─────────────────────────────────────────────────────────────────┐
│                     HYBRID RECORD FORMAT                         │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                      HEADER (16 bytes)                   │    │
│  ├──────────────┬──────────────┬───────────────────────────┤    │
│  │ version (i64)│field_count(u32)│    reserved (u32)       │    │
│  │   8 bytes    │    4 bytes    │       4 bytes            │    │
│  └──────────────┴──────────────┴───────────────────────────┘    │
│                              │                                   │
│                              ▼                                   │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │              FIELD INDEX (N × 16 bytes each)             │    │
│  ├─────────────────────────────────────────────────────────┤    │
│  │  ┌─────────────┬─────────────┬─────────────┬─────────┐  │    │
│  │  │ field_hash  │   offset    │    length   │  type   │  │    │
│  │  │  (u64)      │   (u32)     │    (u32)    │  (u8)   │  │    │
│  │  │  8 bytes    │   4 bytes   │   4 bytes   │ 1 byte  │  │    │
│  │  └─────────────┴─────────────┴─────────────┴─────────┘  │    │
│  │  ... repeated for each field ...                         │    │
│  └─────────────────────────────────────────────────────────┘    │
│                              │                                   │
│                              ▼                                   │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                   FIELD DATA (variable)                  │    │
│  ├─────────────────────────────────────────────────────────┤    │
│  │  [field_0_bytes][field_1_bytes][field_2_bytes]...       │    │
│  │  ◄─── offset points here                                │    │
│  └─────────────────────────────────────────────────────────┘    │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

##### Concrete Example

Consider this user record:
```json
{
  "_spooky_version": 42,
  "id": "user:abc123",
  "name": "Alice",
  "email": "alice@example.com",
  "profile": { "bio": "Developer", "avatar": "https://..." },
  "created_at": "2024-01-15T10:30:00Z"
}
```

**Stored as hybrid format:**

```
Byte Layout (total ~180 bytes for this example):

Offset  | Content                           | Description
--------|-----------------------------------|---------------------------
0-7     | 42 (i64 LE)                       | version
8-11    | 5 (u32 LE)                        | field_count (5 fields)
12-15   | 0                                 | reserved

        === FIELD INDEX (5 × 16 = 80 bytes) ===
        
16-23   | 0x7A3B... (hash of "id")          | field 0: name_hash
24-27   | 96                                | field 0: data offset
28-31   | 12                                | field 0: data length
32      | 1 (String)                        | field 0: type tag

33-40   | 0x9C2D... (hash of "name")        | field 1: name_hash
41-44   | 108                               | field 1: data offset
45-48   | 5                                 | field 1: data length  
49      | 1 (String)                        | field 1: type tag

50-57   | 0x4E8F... (hash of "email")       | field 2: name_hash
58-61   | 113                               | field 2: data offset
62-65   | 17                                | field 2: data length
66      | 1 (String)                        | field 2: type tag

67-74   | 0x1A5C... (hash of "profile")     | field 3: name_hash
75-78   | 130                               | field 3: data offset
79-82   | 45                                | field 3: data length
83      | 5 (Object)                        | field 3: type tag

84-91   | 0xB7E2... (hash of "created_at")  | field 4: name_hash
92-95   | 175                               | field 4: data offset
96-99   | 20                                | field 4: data length
100     | 1 (String)                        | field 4: type tag

        === FIELD DATA (starts at byte 96) ===
        
96-107  | "user:abc123"                     | id value (raw UTF-8)
108-112 | "Alice"                           | name value
113-129 | "alice@example.com"               | email value
130-174 | {"bio":"Developer","avatar":...}  | profile (nested JSON or rkyv)
175-194 | "2024-01-15T10:30:00Z"            | created_at value
```

##### How Field Access Works

**Reading a single field (`email`):**

```rust
fn get_field(record_bytes: &[u8], field_name: &str) -> Option<SpookyValue> {
    // 1. Read header
    let version = i64::from_le_bytes(record_bytes[0..8].try_into().unwrap());
    let field_count = u32::from_le_bytes(record_bytes[8..12].try_into().unwrap());
    
    // 2. Hash the field name we're looking for
    let target_hash = hash_field_name(field_name);  // e.g., hash("email") = 0x4E8F...
    
    // 3. Binary search or linear scan through field index
    let index_start = 16;  // After header
    let index_entry_size = 16;  // 8 + 4 + 4 bytes per entry (ignoring type for simplicity)
    
    for i in 0..field_count as usize {
        let entry_offset = index_start + (i * index_entry_size);
        let field_hash = u64::from_le_bytes(
            record_bytes[entry_offset..entry_offset+8].try_into().unwrap()
        );
        
        if field_hash == target_hash {
            // 4. Found it! Read offset and length
            let data_offset = u32::from_le_bytes(
                record_bytes[entry_offset+8..entry_offset+12].try_into().unwrap()
            ) as usize;
            let data_length = u32::from_le_bytes(
                record_bytes[entry_offset+12..entry_offset+16].try_into().unwrap()
            ) as usize;
            
            // 5. Extract just that field's bytes
            let field_bytes = &record_bytes[data_offset..data_offset+data_length];
            
            // 6. Deserialize only this field
            return Some(deserialize_field(field_bytes));
        }
    }
    None
}
```

**What we avoided:**
- ❌ Parsing the entire JSON
- ❌ Allocating memory for fields we don't need
- ❌ Multiple database reads (unlike Option B)

**What we did:**
- ✅ Single database read
- ✅ ~50 bytes scanned to find field index entry
- ✅ Only deserialize the 17 bytes of email data

##### Type Tags for Optimized Deserialization

The `type` byte in each field index entry allows skipping deserialization for simple types:

```rust
#[repr(u8)]
enum FieldType {
    Null = 0,
    String = 1,      // Raw UTF-8 bytes, no escaping needed
    Int = 2,         // i64 little-endian
    Float = 3,       // f64 little-endian  
    Bool = 4,        // Single byte: 0 or 1
    Object = 5,      // Nested structure (JSON or rkyv sub-record)
    Array = 6,       // Array (JSON or rkyv)
    Binary = 7,      // Raw bytes (for future use)
}

fn deserialize_field(bytes: &[u8], type_tag: FieldType) -> SpookyValue {
    match type_tag {
        FieldType::Null => SpookyValue::Null,
        FieldType::Bool => SpookyValue::Bool(bytes[0] != 0),
        FieldType::Int => {
            let n = i64::from_le_bytes(bytes.try_into().unwrap());
            SpookyValue::Number(n.into())
        }
        FieldType::Float => {
            let n = f64::from_le_bytes(bytes.try_into().unwrap());
            SpookyValue::Number(n.into())
        }
        FieldType::String => {
            // Zero-copy if SpookyValue supports borrowed strings
            let s = std::str::from_utf8(bytes).unwrap();
            SpookyValue::String(s.into())
        }
        FieldType::Object | FieldType::Array => {
            // Fall back to JSON parse for complex types
            serde_json::from_slice(bytes).unwrap()
        }
    }
}
```

##### Complete Table Schema

```
┌─────────────────────────────────────────────────────────────────┐
│ Table: records                                                   │
│ Key: "{table}:{id}"                                             │
│      e.g., "users:user_abc123"                                  │
│ Value: Hybrid format bytes (header + index + data)              │
│                                                                  │
│ Operations:                                                      │
│   • get_record(table, id) → full SpookyValue                    │
│   • get_field(table, id, field) → single field value            │
│   • get_fields(table, id, [fields]) → multiple fields           │
│   • put_record(table, id, SpookyValue) → serialize & store      │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│ Table: zsets                                                     │
│ Key: "{table}:{id}"                                             │
│      e.g., "users:user_abc123"                                  │
│ Value: i64 weight (raw 8 bytes, no serialization)               │
│                                                                  │
│ Operations:                                                      │
│   • get_weight(key) → i64                                       │
│   • update_weight(key, delta) → new_weight                      │
│   • scan_prefix(prefix) → iterator of (key, weight)             │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│ Table: view_cache                                                │
│ Key: "{view_id}"                                                 │
│      e.g., "view_user_list_abc"                                 │
│ Value: Serialized cache structure                                │
│                                                                  │
│ Cache Format:                                                    │
│   ┌─────────────────────────────────────────────────────────┐   │
│   │ [8 bytes: result_count]                                  │   │
│   │ [32 bytes: content_hash (SHA256 or xxHash)]             │   │
│   │ [8 bytes: params_hash]                                   │   │
│   │ [8 bytes: last_updated_timestamp]                        │   │
│   │ [variable: serialized Vec<SpookyValue>]                  │   │
│   └─────────────────────────────────────────────────────────┘   │
│                                                                  │
│ Note: View cache benefits most from rkyv because:               │
│   • Large homogeneous arrays (all same structure)               │
│   • Read-heavy (computed once, read many times)                 │
│   • Known structure at compile time (Vec<SpookyValue>)          │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│ Table: secondary_index (optional, for filtered queries)         │
│ Key: "{table}:idx:{field}:{value_prefix}:{id}"                  │
│      e.g., "users:idx:status:active:user_abc123"                │
│ Value: () empty - key existence is the index                    │
│                                                                  │
│ Use cases:                                                       │
│   • WHERE status = 'active' → prefix scan "users:idx:status:active:"
│   • WHERE age > 18 → requires range-encoded values              │
│                                                                  │
│ Note: Only create indexes for frequently filtered fields.       │
│ Each index adds write overhead (must update on every mutation). │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│ Table: metadata (system table)                                   │
│ Key: Various system keys                                         │
│                                                                  │
│ Entries:                                                         │
│   "schema_version" → u32 (for migrations)                       │
│   "table:{name}:count" → u64 (record count per table)           │
│   "table:{name}:indexes" → JSON list of indexed fields          │
│   "view:{id}:deps" → JSON list of dependent tables              │
└─────────────────────────────────────────────────────────────────┘
```

##### When to Use Each Read Pattern

```
┌────────────────────────────────────────────────────────────────────────┐
│                        READ PATTERN DECISION TREE                       │
├────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  Query needs...                                                         │
│       │                                                                 │
│       ▼                                                                 │
│  ┌─────────────────┐     Yes    ┌─────────────────────────────────┐   │
│  │ All fields?     │───────────►│ get_record() - deserialize full │   │
│  └─────────────────┘            │ ~5µs for 1KB record             │   │
│       │ No                      └─────────────────────────────────┘   │
│       ▼                                                                 │
│  ┌─────────────────┐     Yes    ┌─────────────────────────────────┐   │
│  │ Single field?   │───────────►│ get_field() - index lookup      │   │
│  └─────────────────┘            │ ~500ns regardless of record size│   │
│       │ No                      └─────────────────────────────────┘   │
│       ▼                                                                 │
│  ┌─────────────────┐     Yes    ┌─────────────────────────────────┐   │
│  │ 2-5 fields?     │───────────►│ get_fields() - batch lookup     │   │
│  └─────────────────┘            │ ~200ns per field                │   │
│       │ No (>5 fields)          └─────────────────────────────────┘   │
│       ▼                                                                 │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │ get_record() - at >5 fields, full deserialize is often cheaper │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                                                                         │
└────────────────────────────────────────────────────────────────────────┘
```

##### Implementation: Rust Structures

```rust
use std::borrow::Cow;

/// Header at the start of every hybrid record
#[repr(C, packed)]
struct RecordHeader {
    version: i64,
    field_count: u32,
    _reserved: u32,
}

/// Field index entry (fixed size for easy offset calculation)
#[repr(C, packed)]
struct FieldIndexEntry {
    name_hash: u64,
    data_offset: u32,
    data_length: u32,
    type_tag: u8,
    _padding: [u8; 3],  // Align to 16 bytes
}

/// Zero-copy record reader
pub struct HybridRecord<'a> {
    bytes: &'a [u8],
    header: &'a RecordHeader,
    index: &'a [FieldIndexEntry],
}

impl<'a> HybridRecord<'a> {
    /// Wrap raw bytes as a hybrid record (zero-copy)
    pub fn from_bytes(bytes: &'a [u8]) -> Self {
        let header = unsafe { &*(bytes.as_ptr() as *const RecordHeader) };
        let index_ptr = unsafe { bytes.as_ptr().add(std::mem::size_of::<RecordHeader>()) };
        let index = unsafe {
            std::slice::from_raw_parts(
                index_ptr as *const FieldIndexEntry,
                header.field_count as usize,
            )
        };
        Self { bytes, header, index }
    }
    
    /// Get the record version
    pub fn version(&self) -> i64 {
        self.header.version
    }
    
    /// Get a single field by name (zero-copy for primitives)
    pub fn get_field(&self, name: &str) -> Option<Cow<'a, SpookyValue>> {
        let target_hash = hash_field_name(name);
        
        // Binary search if index is sorted by hash, else linear scan
        let entry = self.index.iter().find(|e| e.name_hash == target_hash)?;
        
        let start = entry.data_offset as usize;
        let end = start + entry.data_length as usize;
        let field_bytes = &self.bytes[start..end];
        
        Some(deserialize_field_cow(field_bytes, entry.type_tag))
    }
    
    /// Get multiple fields efficiently (single pass through index)
    pub fn get_fields(&self, names: &[&str]) -> Vec<Option<SpookyValue>> {
        let target_hashes: Vec<u64> = names.iter().map(|n| hash_field_name(n)).collect();
        let mut results = vec![None; names.len()];
        
        for entry in self.index.iter() {
            if let Some(idx) = target_hashes.iter().position(|&h| h == entry.name_hash) {
                let start = entry.data_offset as usize;
                let end = start + entry.data_length as usize;
                results[idx] = Some(deserialize_field(&self.bytes[start..end], entry.type_tag));
            }
        }
        results
    }
    
    /// Deserialize entire record (when you need all fields)
    pub fn to_spooky_value(&self) -> SpookyValue {
        let mut map = serde_json::Map::new();
        
        for entry in self.index.iter() {
            let start = entry.data_offset as usize;
            let end = start + entry.data_length as usize;
            let value = deserialize_field(&self.bytes[start..end], entry.type_tag);
            let name = reverse_hash_lookup(entry.name_hash); // Need name storage or skip
            map.insert(name, value.into());
        }
        
        SpookyValue::Object(map)
    }
}

/// Serialize a SpookyValue into hybrid format
pub fn serialize_hybrid(value: &SpookyValue, version: i64) -> Vec<u8> {
    let fields: Vec<(&str, &SpookyValue)> = match value {
        SpookyValue::Object(map) => map.iter().map(|(k, v)| (k.as_str(), v)).collect(),
        _ => panic!("Can only serialize objects as records"),
    };
    
    let field_count = fields.len() as u32;
    let header_size = std::mem::size_of::<RecordHeader>();
    let index_size = field_count as usize * std::mem::size_of::<FieldIndexEntry>();
    let data_start = header_size + index_size;
    
    // First pass: serialize all field values to compute offsets
    let mut field_data: Vec<(u64, Vec<u8>, u8)> = Vec::with_capacity(fields.len());
    for (name, value) in &fields {
        let (bytes, type_tag) = serialize_field_value(value);
        field_data.push((hash_field_name(name), bytes, type_tag));
    }
    
    // Calculate total size
    let total_data_size: usize = field_data.iter().map(|(_, b, _)| b.len()).sum();
    let total_size = data_start + total_data_size;
    
    // Allocate and write
    let mut buffer = vec![0u8; total_size];
    
    // Write header
    let header = RecordHeader {
        version,
        field_count,
        _reserved: 0,
    };
    unsafe {
        std::ptr::copy_nonoverlapping(
            &header as *const _ as *const u8,
            buffer.as_mut_ptr(),
            header_size,
        );
    }
    
    // Write index and data
    let mut current_data_offset = data_start;
    for (i, (name_hash, data, type_tag)) in field_data.iter().enumerate() {
        let entry = FieldIndexEntry {
            name_hash: *name_hash,
            data_offset: current_data_offset as u32,
            data_length: data.len() as u32,
            type_tag: *type_tag,
            _padding: [0; 3],
        };
        
        // Write index entry
        let entry_offset = header_size + i * std::mem::size_of::<FieldIndexEntry>();
        unsafe {
            std::ptr::copy_nonoverlapping(
                &entry as *const _ as *const u8,
                buffer.as_mut_ptr().add(entry_offset),
                std::mem::size_of::<FieldIndexEntry>(),
            );
        }
        
        // Write field data
        buffer[current_data_offset..current_data_offset + data.len()].copy_from_slice(data);
        current_data_offset += data.len();
    }
    
    buffer
}

fn hash_field_name(name: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    hasher.finish()
}

fn serialize_field_value(value: &SpookyValue) -> (Vec<u8>, u8) {
    match value {
        SpookyValue::Null => (vec![], 0),
        SpookyValue::Bool(b) => (vec![if *b { 1 } else { 0 }], 4),
        SpookyValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                (i.to_le_bytes().to_vec(), 2)
            } else {
                (n.as_f64().unwrap().to_le_bytes().to_vec(), 3)
            }
        }
        SpookyValue::String(s) => (s.as_bytes().to_vec(), 1),
        SpookyValue::Array(_) | SpookyValue::Object(_) => {
            // Fall back to JSON for complex types
            (serde_json::to_vec(value).unwrap(), 5)
        }
    }
}
```

##### Performance Comparison

| Operation | Option A (Whole) | Option B (Field-Level) | Option C (Hybrid) |
|-----------|------------------|------------------------|-------------------|
| Get full record | ✅ 1 read, 1 deserialize | ❌ N reads, N deserialize | ✅ 1 read, 1 deserialize |
| Get 1 field | ❌ 1 read, full deserialize | ✅ 1 read, 1 deserialize | ✅ 1 read, partial scan |
| Get 3 fields | ❌ 1 read, full deserialize | ⚠️ 3 reads | ✅ 1 read, partial scan |
| Update 1 field | ✅ Read-modify-write | ✅ 1 write | ⚠️ Read-modify-write |
| Storage overhead | None | High (key per field) | Low (~16 bytes/field index) |
| Implementation | Simple | Complex | Medium |

##### Why Hybrid Wins for SSP

1. **Your query patterns**: Views often need specific fields (e.g., `SELECT id, name FROM users WHERE status = 'active'`)

2. **Record sizes are moderate**: Typical 200B-2KB records where full deserialize isn't catastrophic but partial reads help

3. **Single key = simpler transactions**: Batch writes update one key per record, not N keys per field

4. **Iteration is common**: Table scans for view computation benefit from single-key-per-record

5. **Future-proof for rkyv**: The hybrid format is essentially what rkyv does internally - you're building the same structure manually, which makes rkyv migration straightforward later

---

##### When to Use rkyv in the Hybrid Approach

The hybrid format can be implemented with **manual serialization** (as shown above) or with **rkyv**. Here's a detailed guide on when rkyv adds value versus when it's overhead.

###### Understanding rkyv's Value Proposition

rkyv (Rust Archive) provides **zero-copy deserialization** - you can read data directly from bytes without copying or parsing. But this comes with trade-offs:

```
┌─────────────────────────────────────────────────────────────────────────┐
│                    SERIALIZATION COMPARISON                              │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  JSON Deserialization:                                                   │
│  ┌──────────────┐     Parse      ┌──────────────┐    Allocate    ┌────┐│
│  │ Disk/Memory  │ ───────────► │   Tokens     │ ────────────► │Heap ││
│  │   Bytes      │    O(n)       │              │     O(n)      │Copy ││
│  └──────────────┘               └──────────────┘               └────┘│
│  Total: ~5µs for 1KB record                                          │
│                                                                          │
│  rkyv Zero-Copy:                                                        │
│  ┌──────────────┐    Validate    ┌──────────────┐                      │
│  │ Disk/Memory  │ ───────────► │ Direct Use   │  (no allocation)      │
│  │   Bytes      │    O(1)*      │ via &ref     │                       │
│  └──────────────┘               └──────────────┘                       │
│  Total: ~100ns for 1KB record                                          │
│                                                                          │
│  * O(1) with unsafe, O(n) with validation enabled                       │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

###### Decision Matrix: When to Use rkyv

```
┌────────────────────────────────────────────────────────────────────────────┐
│                         RKYV DECISION MATRIX                                │
├────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│                              Record Size                                    │
│              ┌─────────────┬─────────────┬─────────────┬─────────────┐     │
│              │   < 256B    │  256B-1KB   │   1KB-10KB  │    > 10KB   │     │
│  ┌───────────┼─────────────┼─────────────┼─────────────┼─────────────┤     │
│  │Read Full  │ ❌ JSON     │ ⚠️ Either   │ ✅ rkyv    │ ✅ rkyv    │     │
│  │Record     │ (fast enough)│            │ (50x faster)│ (100x faster)│    │
│  ├───────────┼─────────────┼─────────────┼─────────────┼─────────────┤     │
│  │Read 1-2   │ ❌ JSON     │ ✅ rkyv    │ ✅ rkyv    │ ✅ rkyv    │     │
│  │Fields     │             │ (10x faster)│ (50x faster)│              │     │
│  ├───────────┼─────────────┼─────────────┼─────────────┼─────────────┤     │
│  │Frequent   │ ❌ JSON     │ ❌ JSON     │ ⚠️ Either   │ ⚠️ Either   │     │
│  │Writes     │(no overhead)│(serialize cost)│           │              │     │
│  ├───────────┼─────────────┼─────────────┼─────────────┼─────────────┤     │
│  │View Cache │ ⚠️ Either   │ ✅ rkyv    │ ✅ rkyv    │ ✅ rkyv    │     │
│  │(arrays)   │             │ (huge win)  │ (huge win)  │ (huge win)  │     │
│  ├───────────┼─────────────┼─────────────┼─────────────┼─────────────┤     │
│  │WASM       │ ❌ JSON     │ ❌ JSON     │ ⚠️ rkyv*   │ ⚠️ rkyv*   │     │
│  │Target     │             │             │ *reduced benefit          │     │
│  └───────────┴─────────────┴─────────────┴─────────────┴─────────────┘     │
│                                                                             │
│  Legend: ❌ Don't use rkyv  ⚠️ Measure first  ✅ Use rkyv                  │
│                                                                             │
└────────────────────────────────────────────────────────────────────────────┘
```

###### rkyv for Different Data Types in SSP

| Data Type | Typical Size | Read Pattern | Recommendation |
|-----------|--------------|--------------|----------------|
| User records | 200-500B | Full record, frequent | **JSON** - small, dynamic schema |
| Thread records | 300-800B | Full + field projection | **Measure** - borderline |
| Message content | 1-50KB | Often just preview field | **rkyv** - large, partial reads |
| Embedded media metadata | 2-20KB | Specific fields only | **rkyv** - large, sparse access |
| View cache (100 records) | 20-200KB | Full array iteration | **rkyv** - huge arrays, read-heavy |
| View cache (1000 records) | 200KB-2MB | Full array iteration | **rkyv** - absolutely essential |
| ZSet weights | 8B | Single i64 | **Raw bytes** - no serialization |
| Secondary indexes | 0B (key only) | Existence check | **None** - just key presence |

###### Concrete Performance Numbers

Based on typical SSP workloads:

```rust
// Benchmark: Reading a single field from a 1KB record

// JSON approach
fn get_email_json(bytes: &[u8]) -> String {
    let record: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    record["email"].as_str().unwrap().to_string()
}
// Time: ~4,500 ns (4.5 µs)
// Allocations: 15-20 (parsing + string copies)

// Manual hybrid approach (from earlier in this doc)  
fn get_email_hybrid(bytes: &[u8]) -> String {
    let record = HybridRecord::from_bytes(bytes);
    record.get_field("email").unwrap().as_str().to_string()
}
// Time: ~800 ns
// Allocations: 1 (final string copy)

// rkyv approach
fn get_email_rkyv(bytes: &[u8]) -> &str {
    let archived = unsafe { rkyv::archived_root::<Record>(bytes) };
    &archived.email  // Zero-copy reference!
}
// Time: ~50 ns
// Allocations: 0
```

**The 90x speedup** from JSON to rkyv matters when:
- You're doing thousands of field accesses per second
- You're iterating over large view caches
- Latency is critical (realtime updates)

**It doesn't matter when:**
- You're reading < 100 records/second
- The record is small (< 256 bytes)
- You need the full record anyway and it's small

###### rkyv Implementation for Hybrid Format

Here's how to integrate rkyv into the hybrid approach:

```rust
use rkyv::{Archive, Deserialize, Serialize, archived_root, to_bytes};
use rkyv::ser::serializers::AllocSerializer;

// Define archived versions of your types
#[derive(Archive, Deserialize, Serialize, Debug)]
#[archive(check_bytes)]  // Enable validation (recommended for untrusted data)
pub struct ArchivedHybridRecord {
    pub version: i64,
    pub fields: Vec<ArchivedField>,
}

#[derive(Archive, Deserialize, Serialize, Debug)]
pub struct ArchivedField {
    pub name_hash: u64,
    pub value: ArchivedFieldValue,
}

#[derive(Archive, Deserialize, Serialize, Debug)]
pub enum ArchivedFieldValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    // For nested objects, store as JSON bytes (pragmatic compromise)
    Json(Vec<u8>),
}

impl ArchivedHybridRecord {
    /// Zero-copy field access
    pub fn get_field(&self, name: &str) -> Option<&ArchivedFieldValue> {
        let target_hash = hash_field_name(name);
        self.fields
            .iter()
            .find(|f| f.name_hash == target_hash)
            .map(|f| &f.value)
    }
}

// Serialization
pub fn serialize_record_rkyv(value: &SpookyValue, version: i64) -> Vec<u8> {
    let fields: Vec<ArchivedField> = match value {
        SpookyValue::Object(map) => {
            map.iter().map(|(k, v)| ArchivedField {
                name_hash: hash_field_name(k),
                value: spooky_to_archived(v),
            }).collect()
        }
        _ => panic!("Records must be objects"),
    };
    
    let record = ArchivedHybridRecord { version, fields };
    
    // Serialize with rkyv
    to_bytes::<_, 1024>(&record)  // 1024 = scratch space hint
        .expect("serialization failed")
        .into_vec()
}

// Zero-copy deserialization
pub fn read_record_rkyv(bytes: &[u8]) -> &ArchivedArchivedHybridRecord {
    // SAFETY: Data must be valid rkyv archive
    // Use check_bytes feature for validation in untrusted contexts
    unsafe { archived_root::<ArchivedHybridRecord>(bytes) }
}

// Safe version with validation (slower but safe for untrusted data)
pub fn read_record_rkyv_safe(bytes: &[u8]) -> Result<&ArchivedArchivedHybridRecord, rkyv::validation::CheckArchiveError> {
    rkyv::check_archived_root::<ArchivedHybridRecord>(bytes)
}

fn spooky_to_archived(v: &SpookyValue) -> ArchivedFieldValue {
    match v {
        SpookyValue::Null => ArchivedFieldValue::Null,
        SpookyValue::Bool(b) => ArchivedFieldValue::Bool(*b),
        SpookyValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                ArchivedFieldValue::Int(i)
            } else {
                ArchivedFieldValue::Float(n.as_f64().unwrap())
            }
        }
        SpookyValue::String(s) => ArchivedFieldValue::String(s.clone()),
        // Nested objects: fall back to JSON (pragmatic)
        SpookyValue::Array(_) | SpookyValue::Object(_) => {
            ArchivedFieldValue::Json(serde_json::to_vec(v).unwrap())
        }
    }
}
```

###### The Pragmatic Compromise: Nested Objects

rkyv works best with **statically known structures**. Your `SpookyValue` is dynamically typed (like `serde_json::Value`), which is awkward for rkyv.

**Three approaches for nested objects:**

1. **Store nested as JSON bytes** (recommended for SSP)
   ```rust
   ArchivedFieldValue::Json(Vec<u8>)  // Parse on access if needed
   ```
   - ✅ Simple, handles any nesting
   - ✅ Top-level fields still get zero-copy
   - ❌ Nested access requires JSON parse

2. **Recursive archive structure**
   ```rust
   enum ArchivedFieldValue {
       Object(Vec<(u64, ArchivedFieldValue)>),  // Recursive
       Array(Vec<ArchivedFieldValue>),
       // ...
   }
   ```
   - ✅ Full zero-copy for everything
   - ❌ Complex implementation
   - ❌ Schema changes are painful

3. **Flatten on write, reconstruct on read**
   ```rust
   // Store: user.profile.bio → field "profile.bio"
   fields: [
       ("profile.bio", "Developer"),
       ("profile.avatar", "https://..."),
   ]
   ```
   - ✅ All fields are top-level, all get zero-copy
   - ❌ Reconstructing nested structure is complex
   - ❌ Array indexing becomes weird ("items.0.name")

**Recommendation for SSP:** Use approach #1. Most queries access top-level fields. When you need nested data, the JSON parse is still faster than parsing the entire record.

###### View Cache: Where rkyv Shines Most

View caches are the **highest-value target** for rkyv in SSP:

```rust
#[derive(Archive, Deserialize, Serialize)]
pub struct ArchivedViewCache {
    pub result_count: u64,
    pub content_hash: [u8; 32],
    pub params_hash: u64,
    pub last_updated: i64,
    pub results: Vec<ArchivedCacheRecord>,  // This is the big win
}

#[derive(Archive, Deserialize, Serialize)]
pub struct ArchivedCacheRecord {
    pub id: String,
    pub hash: u64,
    pub data: Vec<u8>,  // Individual record bytes (could be rkyv or JSON)
}
```

**Why view cache benefits most:**

| Factor | Impact |
|--------|--------|
| Size | Caches can be 100KB-10MB (1000s of records) |
| Read pattern | Always iterate full array |
| Write frequency | Only on view update (relatively rare) |
| Structure | Homogeneous array (all same type) |
| Lifetime | Long-lived, read many times |

**Performance example:**

```rust
// View cache with 1000 records, ~500KB total

// JSON approach
fn load_cache_json(bytes: &[u8]) -> Vec<CacheRecord> {
    serde_json::from_slice(bytes).unwrap()
}
// Time: ~25ms
// Allocations: ~3000 (vec + 1000 records + strings)
// Memory: 500KB copied to heap

// rkyv approach  
fn load_cache_rkyv(bytes: &[u8]) -> &ArchivedVec<ArchivedCacheRecord> {
    let archived = unsafe { archived_root::<ArchivedViewCache>(bytes) };
    &archived.results
}
// Time: ~1µs (just pointer cast)
// Allocations: 0
// Memory: 0 (reads directly from mmap'd file)

// Iteration is also faster with rkyv
fn iterate_cache_rkyv(bytes: &[u8]) {
    let cache = unsafe { archived_root::<ArchivedViewCache>(bytes) };
    for record in cache.results.iter() {
        // record is &ArchivedCacheRecord - zero copy
        process(&record.id, record.hash);
    }
}
// 1000 iterations: ~50µs

fn iterate_cache_json(cache: &[CacheRecord]) {
    for record in cache {
        process(&record.id, record.hash);
    }
}
// 1000 iterations: ~100µs (still need to access heap-allocated data)
```

###### Migration Strategy: JSON → Hybrid → rkyv

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        MIGRATION PHASES                                  │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  Phase 1: JSON Baseline (Current)                                        │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ • SpookyValue stored as JSON bytes                               │    │
│  │ • Simple, debuggable, flexible schema                            │    │
│  │ • Baseline for performance comparison                            │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                              │                                           │
│                              ▼                                           │
│  Phase 2: Manual Hybrid (Recommended First Step)                         │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ • Implement HybridRecord format from this doc                    │    │
│  │ • Field index enables partial reads                              │    │
│  │ • Still uses JSON for field values (simple)                      │    │
│  │ • No external dependencies                                       │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                              │                                           │
│                              ▼                                           │
│  Phase 3: rkyv for View Cache (High Value, Low Risk)                    │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ • Keep records as hybrid/JSON                                    │    │
│  │ • Use rkyv ONLY for view_cache table                            │    │
│  │ • Biggest performance win with minimal complexity               │    │
│  │ • View cache structure is stable (won't change often)           │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                              │                                           │
│                              ▼                                           │
│  Phase 4: rkyv for Large Records (Only If Needed)                       │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ • Profile shows record deserialization is bottleneck             │    │
│  │ • Average record size > 1KB                                      │    │
│  │ • Add rkyv for records table                                     │    │
│  │ • Keep JSON fallback for debugging                               │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

###### rkyv Gotchas and Pitfalls

| Gotcha | Impact | Mitigation |
|--------|--------|------------|
| Schema evolution | Adding/removing fields breaks existing archives | Version field + migration code |
| Endianness | Archives are platform-specific by default | Use `#[archive(archived = "...")]` for portability |
| Unsafe by default | `archived_root` is unsafe | Use `check_archived_root` for untrusted data |
| No human readability | Can't inspect with text editor | Keep JSON debug dump option |
| Alignment requirements | Some platforms need aligned reads | rkyv handles this, but be aware |
| Nested dynamic types | `Vec<Value>` is awkward | Use JSON fallback for nested objects |
| WASM memory model | No mmap, less benefit | Still faster than JSON, but gap is smaller |

###### Summary: rkyv Recommendations for SSP

| Component | Use rkyv? | When to Implement |
|-----------|-----------|-------------------|
| Records (< 512B) | ❌ No | Never |
| Records (> 512B) | ⚠️ Maybe | Phase 4, only if profiling shows need |
| View cache | ✅ Yes | Phase 3, high priority |
| ZSet weights | ❌ No | Never (raw i64 is fine) |
| Secondary indexes | ❌ No | Never (key-only) |
| Metadata | ❌ No | Never (human-readable preferred) |

**Bottom line:** Start with manual hybrid format for records, add rkyv for view cache when you need it, and only consider rkyv for records if profiling shows deserialization is a bottleneck.

---

##### Critical Analysis: I/O vs CPU Trade-offs

**Important realization:** The hybrid format (Option C) optimizes for **CPU parsing time**, not **disk I/O**. When reading from redb, you must load the entire value before you can access any field.

```
┌─────────────────────────────────────────────────────────────────────────┐
│            THE HIDDEN COST OF HYBRID FORMAT                              │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  To read ONE field ("email") from a 2KB hybrid record in redb:          │
│                                                                          │
│  Step 1: redb.get("users:abc123")                                       │
│          └─► Database reads ENTIRE 2KB value from disk                  │
│          └─► This is unavoidable with key-value stores                  │
│                                                                          │
│  Step 2: Parse header (16 bytes)                    ✓ Fast              │
│  Step 3: Scan field index (160 bytes)               ✓ Fast              │
│  Step 4: Jump to offset, read email (20 bytes)      ✓ Fast              │
│  Step 5: Deserialize only email field               ✓ Fast              │
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │  PROBLEM: Step 1 already loaded 2KB from disk!                   │    │
│  │  Steps 2-5 save CPU time, but I/O damage is already done.       │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

###### Actual I/O Comparison Across All Options

| Approach | Read 1 Field | Read Full Record | Write 1 Field | Storage Overhead |
|----------|--------------|------------------|---------------|------------------|
| **Option A: Whole JSON** | 2KB read, 2KB parse | 2KB read, 2KB parse | 2KB write | None |
| **Option B: Field-per-key** | **20B read** ✅ | 2KB (N fetches) | 20B write | High (N keys) |
| **Option C: Hybrid** | 2KB read, 20B parse | 2KB read, 2KB parse | 2KB write | Low (~160B) |
| **Option C + rkyv** | 2KB read, zero-copy | 2KB read, zero-copy | 2KB write | Low |

**Key insight:** Option C (Hybrid) only wins when:
- Data is already in memory/page cache (no disk I/O)
- CPU parsing is the bottleneck
- You're reading multiple fields from the same record

Option B wins when:
- Disk I/O is the bottleneck
- You consistently need only 1-2 fields
- Working set exceeds available memory

###### Real-World Bottleneck Analysis

```
┌────────────────────────────────────────────────────────────────────────────┐
│                    WHERE IS YOUR ACTUAL BOTTLENECK?                         │
├────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  Scenario 1: DISK I/O BOUND                                                │
│  ┌───────────────────────────────────────────────────────────────────┐     │
│  │ Symptoms:                                                          │     │
│  │ • Slow SSD or spinning disk                                        │     │
│  │ • Dataset larger than available RAM                                │     │
│  │ • High disk wait times in profiler                                 │     │
│  │ • redb file not fitting in page cache                             │     │
│  │                                                                    │     │
│  │ Winner: Option B (field-per-key)                                   │     │
│  │ • 20B read vs 2KB read = 100x less I/O                            │     │
│  │ • Worth the key-space overhead                                     │     │
│  │ • Or: Option D (hot/cold split) - see below                       │     │
│  └───────────────────────────────────────────────────────────────────┘     │
│                                                                             │
│  Scenario 2: CPU/PARSING BOUND                                             │
│  ┌───────────────────────────────────────────────────────────────────┐     │
│  │ Symptoms:                                                          │     │
│  │ • Fast NVMe SSD                                                    │     │
│  │ • Dataset fits in RAM / page cache                                 │     │
│  │ • High CPU usage during queries                                    │     │
│  │ • Flamegraph shows time in serde_json::from_slice                 │     │
│  │                                                                    │     │
│  │ Winner: Option C (Hybrid) + rkyv                                   │     │
│  │ • I/O is effectively free (cached)                                │     │
│  │ • Zero-copy parsing eliminates allocations                        │     │
│  │ • Field index avoids parsing unused data                          │     │
│  └───────────────────────────────────────────────────────────────────┘     │
│                                                                             │
│  Scenario 3: MEMORY BOUND                                                  │
│  ┌───────────────────────────────────────────────────────────────────┐     │
│  │ Symptoms:                                                          │     │
│  │ • Limited RAM (embedded, mobile, WASM)                            │     │
│  │ • Many concurrent views                                            │     │
│  │ • Memory pressure / GC pauses                                      │     │
│  │                                                                    │     │
│  │ Winner: Option B or Option D                                       │     │
│  │ • Smaller working set per query                                   │     │
│  │ • Better cache line utilization                                   │     │
│  │ • Less allocation pressure                                        │     │
│  └───────────────────────────────────────────────────────────────────┘     │
│                                                                             │
│  Scenario 4: WRITE THROUGHPUT BOUND                                        │
│  ┌───────────────────────────────────────────────────────────────────┐     │
│  │ Symptoms:                                                          │     │
│  │ • High mutation rate (>10K writes/sec)                            │     │
│  │ • Write latency spikes                                            │     │
│  │ • redb compaction overhead                                        │     │
│  │                                                                    │     │
│  │ Winner: Option A or C (whole record per key)                      │     │
│  │ • Single key write vs N key writes                                │     │
│  │ • Less write amplification                                        │     │
│  │ • Simpler transaction handling                                    │     │
│  └───────────────────────────────────────────────────────────────────┘     │
│                                                                             │
└────────────────────────────────────────────────────────────────────────────┘
```

---

#### Option D: Hot/Cold Split (New Recommendation)

Given the I/O vs CPU trade-off analysis, **Option D** emerges as the best approach for SSP's specific workload. It combines the benefits of whole-record storage with minimal I/O for common operations.

##### Core Concept: Separate Hot and Cold Data

```
┌─────────────────────────────────────────────────────────────────────────┐
│                    OPTION D: HOT/COLD SPLIT ARCHITECTURE                 │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  The insight: DBSP operations rarely need full records.                 │
│  Most operations need only:                                              │
│  • Record ID (for ZSet membership)                                      │
│  • Version (for change detection)                                       │
│  • Hash (for content change detection)                                  │
│  • 2-3 commonly filtered fields (status, updated_at, etc.)             │
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │                         redb Tables                              │    │
│  ├─────────────────────────────────────────────────────────────────┤    │
│  │                                                                  │    │
│  │  HOT TABLE (record_hot) ──────────────────────────────────────  │    │
│  │  Key: "{table}:{id}"                                            │    │
│  │  Value: 48-128 bytes (fixed size, rkyv)                         │    │
│  │  ┌────────────────────────────────────────────────────────┐     │    │
│  │  │ • id (SmolStr)           - 24 bytes                    │     │    │
│  │  │ • version (i64)          - 8 bytes                     │     │    │
│  │  │ • content_hash (u64)     - 8 bytes                     │     │    │
│  │  │ • hot_field_0 (status)   - variable                    │     │    │
│  │  │ • hot_field_1 (updated)  - 8 bytes                     │     │    │
│  │  │ • ... up to 5 hot fields                               │     │    │
│  │  └────────────────────────────────────────────────────────┘     │    │
│  │  Read: ~100 bytes for most DBSP operations                      │    │
│  │                                                                  │    │
│  │  COLD TABLE (record_cold) ────────────────────────────────────  │    │
│  │  Key: "{table}:{id}"                                            │    │
│  │  Value: Full record (JSON or hybrid format)                     │    │
│  │  ┌────────────────────────────────────────────────────────┐     │    │
│  │  │ • Complete SpookyValue with all fields                 │     │    │
│  │  │ • 500B - 50KB typical                                  │     │    │
│  │  └────────────────────────────────────────────────────────┘     │    │
│  │  Read: Only when view needs full record or non-hot fields       │    │
│  │                                                                  │    │
│  │  ZSETS TABLE ─────────────────────────────────────────────────  │    │
│  │  Key: "{table}:{id}"                                            │    │
│  │  Value: i64 weight                                              │    │
│  │                                                                  │    │
│  │  VIEW_CACHE TABLE ────────────────────────────────────────────  │    │
│  │  Key: "{view_id}"                                               │    │
│  │  Value: rkyv serialized cache                                   │    │
│  │                                                                  │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

##### Why Hot/Cold Split Works for SSP

```
┌─────────────────────────────────────────────────────────────────────────┐
│                     SSP OPERATION ANALYSIS                               │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  Operation                    │ Fields Needed      │ Hot Table Enough?  │
│  ────────────────────────────│───────────────────│───────────────────  │
│  ZSet membership check        │ just key existence │ ✅ Yes (ZSet table)│
│  Delta propagation            │ id, version        │ ✅ Yes             │
│  Change detection             │ content_hash       │ ✅ Yes             │
│  WHERE status = 'active'      │ status             │ ✅ Yes (if hot)    │
│  WHERE updated_at > X         │ updated_at         │ ✅ Yes (if hot)    │
│  SELECT id, name, email       │ id + 2 fields      │ ⚠️ Maybe          │
│  SELECT * FROM users          │ all fields         │ ❌ No (need cold)  │
│  Subquery: get author name    │ specific fields    │ ⚠️ Depends        │
│                                                                          │
│  Estimated hot table hit rate for typical DBSP workload: 70-85%         │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

##### Hot Record Structure

```rust
use rkyv::{Archive, Deserialize, Serialize};
use smol_str::SmolStr;

/// Fixed-size hot record for fast access
/// Total size: 64-128 bytes depending on configuration
#[derive(Archive, Deserialize, Serialize, Clone, Debug)]
#[archive(check_bytes)]
pub struct HotRecord {
    // === Core DBSP fields (always present) === //
    
    /// Record version for optimistic/authoritative update tracking
    pub version: i64,
    
    /// Hash of full record content for change detection
    /// Computed as: xxhash64(canonical_json(full_record))
    pub content_hash: u64,
    
    /// Timestamp of last modification (Unix millis)
    pub updated_at: i64,
    
    // === Configurable hot fields (table-specific) === //
    
    /// Up to 5 frequently-accessed fields stored inline
    /// Configuration is per-table in TableSchema
    pub hot_fields: HotFields,
}

/// Table-specific hot fields
/// Each table can configure which fields are "hot"
#[derive(Archive, Deserialize, Serialize, Clone, Debug)]
pub enum HotFields {
    /// No additional hot fields (just core DBSP fields)
    None,
    
    /// Generic: up to 5 named fields with dynamic values
    Generic {
        fields: [(u64, HotFieldValue); 5],  // (name_hash, value)
        field_count: u8,
    },
    
    /// User table optimized layout
    User {
        status: HotFieldValue,      // active/inactive/banned
        role: HotFieldValue,        // admin/user/guest
    },
    
    /// Thread/Post table optimized layout  
    Thread {
        status: HotFieldValue,      // draft/published/archived
        author_id: SmolStr,         // For JOIN optimization
        category_id: SmolStr,       // For filtering
    },
    
    /// Message table optimized layout
    Message {
        thread_id: SmolStr,         // Parent reference
        sender_id: SmolStr,         // For filtering
        is_read: bool,              // Common filter
    },
}

/// Compact field value representation
#[derive(Archive, Deserialize, Serialize, Clone, Debug)]
pub enum HotFieldValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// Short strings stored inline (up to 23 bytes with SmolStr)
    String(SmolStr),
    /// Reference to full record for larger values
    /// Signals: "this field exists but fetch from cold storage"
    InCold,
}

impl HotRecord {
    /// Size in bytes when serialized with rkyv
    /// Predictable size enables better cache utilization
    pub const MAX_SIZE: usize = 128;
    
    /// Check if a field is available in hot storage
    pub fn has_hot_field(&self, field_name: &str) -> bool {
        let hash = hash_field_name(field_name);
        match &self.hot_fields {
            HotFields::None => false,
            HotFields::Generic { fields, field_count } => {
                fields[..*field_count as usize]
                    .iter()
                    .any(|(h, _)| *h == hash)
            }
            // Table-specific variants have known fields
            HotFields::User { .. } => {
                matches!(field_name, "status" | "role")
            }
            HotFields::Thread { .. } => {
                matches!(field_name, "status" | "author_id" | "category_id")
            }
            HotFields::Message { .. } => {
                matches!(field_name, "thread_id" | "sender_id" | "is_read")
            }
        }
    }
    
    /// Get a hot field value
    pub fn get_hot_field(&self, field_name: &str) -> Option<&HotFieldValue> {
        match &self.hot_fields {
            HotFields::None => None,
            HotFields::Generic { fields, field_count } => {
                let hash = hash_field_name(field_name);
                fields[..*field_count as usize]
                    .iter()
                    .find(|(h, _)| *h == hash)
                    .map(|(_, v)| v)
            }
            HotFields::User { status, role } => match field_name {
                "status" => Some(status),
                "role" => Some(role),
                _ => None,
            },
            HotFields::Thread { status, author_id, category_id } => match field_name {
                "status" => Some(status),
                "author_id" => Some(&HotFieldValue::String(author_id.clone())),
                "category_id" => Some(&HotFieldValue::String(category_id.clone())),
                _ => None,
            },
            // ... etc
        }
    }
}
```

##### Table Schema Configuration

```rust
/// Per-table configuration for hot/cold split
#[derive(Clone, Debug)]
pub struct TableSchema {
    pub name: SmolStr,
    
    /// Fields to include in hot record (besides core DBSP fields)
    /// Order matters: first fields are most likely to be accessed
    pub hot_fields: Vec<HotFieldConfig>,
    
    /// Maximum number of hot fields (default: 5)
    pub max_hot_fields: usize,
    
    /// Fields that should trigger content_hash recalculation
    /// Default: all fields. Set to specific fields for optimization.
    pub hash_fields: HashFieldsConfig,
}

#[derive(Clone, Debug)]
pub struct HotFieldConfig {
    pub name: SmolStr,
    /// Maximum size for this field in hot storage
    /// Larger values get InCold marker
    pub max_size: usize,
    /// Whether this field is commonly used in WHERE clauses
    pub is_filter_field: bool,
}

#[derive(Clone, Debug)]
pub enum HashFieldsConfig {
    /// Hash all fields (default, safest)
    All,
    /// Hash only specific fields (optimization)
    /// Use when some fields change frequently but don't affect views
    Only(Vec<SmolStr>),
    /// Hash all except specific fields
    Except(Vec<SmolStr>),
}

// Example configurations
impl TableSchema {
    pub fn users() -> Self {
        Self {
            name: SmolStr::new("users"),
            hot_fields: vec![
                HotFieldConfig { 
                    name: SmolStr::new("status"), 
                    max_size: 16,
                    is_filter_field: true,
                },
                HotFieldConfig { 
                    name: SmolStr::new("role"), 
                    max_size: 16,
                    is_filter_field: true,
                },
                HotFieldConfig { 
                    name: SmolStr::new("name"), 
                    max_size: 64,
                    is_filter_field: false,
                },
            ],
            max_hot_fields: 5,
            hash_fields: HashFieldsConfig::Except(vec![
                SmolStr::new("last_seen_at"),  // Changes often, doesn't affect views
            ]),
        }
    }
    
    pub fn messages() -> Self {
        Self {
            name: SmolStr::new("messages"),
            hot_fields: vec![
                HotFieldConfig { 
                    name: SmolStr::new("thread_id"), 
                    max_size: 32,
                    is_filter_field: true,
                },
                HotFieldConfig { 
                    name: SmolStr::new("sender_id"), 
                    max_size: 32,
                    is_filter_field: true,
                },
                HotFieldConfig { 
                    name: SmolStr::new("is_read"), 
                    max_size: 1,
                    is_filter_field: true,
                },
            ],
            max_hot_fields: 5,
            hash_fields: HashFieldsConfig::All,
        }
    }
}
```

##### Storage Implementation

```rust
use redb::{Database, ReadableTable, TableDefinition, WriteTransaction};

const HOT_RECORDS: TableDefinition<&str, &[u8]> = TableDefinition::new("record_hot");
const COLD_RECORDS: TableDefinition<&str, &[u8]> = TableDefinition::new("record_cold");
const ZSETS: TableDefinition<&str, i64> = TableDefinition::new("zsets");
const VIEW_CACHE: TableDefinition<&str, &[u8]> = TableDefinition::new("view_cache");

pub struct HotColdStorage {
    db: Database,
    schemas: FastMap<SmolStr, TableSchema>,
}

impl HotColdStorage {
    pub fn new(path: &Path, schemas: Vec<TableSchema>) -> Result<Self> {
        let db = Database::create(path)?;
        let schemas = schemas.into_iter()
            .map(|s| (s.name.clone(), s))
            .collect();
        Ok(Self { db, schemas })
    }
    
    /// Get hot record only (fast path)
    /// Returns None if record doesn't exist
    pub fn get_hot(&self, table: &str, id: &str) -> Option<HotRecord> {
        let key = format!("{}:{}", table, id);
        let read_txn = self.db.begin_read().ok()?;
        let hot_table = read_txn.open_table(HOT_RECORDS).ok()?;
        let bytes = hot_table.get(key.as_str()).ok()??;
        
        // Zero-copy access with rkyv
        let archived = unsafe { 
            rkyv::archived_root::<HotRecord>(bytes.value()) 
        };
        // Deserialize only if needed (often can use archived directly)
        Some(archived.deserialize(&mut rkyv::Infallible).unwrap())
    }
    
    /// Get specific fields, using hot or cold as needed
    pub fn get_fields(
        &self, 
        table: &str, 
        id: &str, 
        fields: &[&str]
    ) -> Option<SpookyValue> {
        // Check if all requested fields are in hot storage
        let schema = self.schemas.get(table)?;
        let all_hot = fields.iter().all(|f| {
            *f == "id" || 
            *f == "_spooky_version" ||
            schema.hot_fields.iter().any(|hf| hf.name == *f)
        });
        
        if all_hot {
            // Fast path: read from hot table only
            let hot = self.get_hot(table, id)?;
            Some(self.hot_to_partial_spooky(&hot, fields))
        } else {
            // Slow path: need cold record
            self.get_cold(table, id)
        }
    }
    
    /// Get full record (cold storage)
    pub fn get_cold(&self, table: &str, id: &str) -> Option<SpookyValue> {
        let key = format!("{}:{}", table, id);
        let read_txn = self.db.begin_read().ok()?;
        let cold_table = read_txn.open_table(COLD_RECORDS).ok()?;
        let bytes = cold_table.get(key.as_str()).ok()??;
        
        // Cold storage uses JSON for flexibility
        serde_json::from_slice(bytes.value()).ok()
    }
    
    /// Write record to both hot and cold storage
    pub fn put_record(
        &self, 
        table: &str, 
        id: &str, 
        data: &SpookyValue,
        version: i64,
    ) -> Result<()> {
        let key = format!("{}:{}", table, id);
        let schema = self.schemas.get(table)
            .cloned()
            .unwrap_or_else(|| TableSchema::default());
        
        // Build hot record
        let hot = self.build_hot_record(data, version, &schema);
        let hot_bytes = rkyv::to_bytes::<_, 256>(&hot)?;
        
        // Serialize cold record (JSON)
        let cold_bytes = serde_json::to_vec(data)?;
        
        // Write both atomically
        let write_txn = self.db.begin_write()?;
        {
            let mut hot_table = write_txn.open_table(HOT_RECORDS)?;
            let mut cold_table = write_txn.open_table(COLD_RECORDS)?;
            
            hot_table.insert(key.as_str(), hot_bytes.as_slice())?;
            cold_table.insert(key.as_str(), cold_bytes.as_slice())?;
        }
        write_txn.commit()?;
        
        Ok(())
    }
    
    /// Delete record from both hot and cold storage
    pub fn delete_record(&self, table: &str, id: &str) -> Result<()> {
        let key = format!("{}:{}", table, id);
        
        let write_txn = self.db.begin_write()?;
        {
            let mut hot_table = write_txn.open_table(HOT_RECORDS)?;
            let mut cold_table = write_txn.open_table(COLD_RECORDS)?;
            
            hot_table.remove(key.as_str())?;
            cold_table.remove(key.as_str())?;
        }
        write_txn.commit()?;
        
        Ok(())
    }
    
    /// Build hot record from full SpookyValue
    fn build_hot_record(
        &self, 
        data: &SpookyValue, 
        version: i64,
        schema: &TableSchema,
    ) -> HotRecord {
        let content_hash = self.compute_content_hash(data, schema);
        let updated_at = data.get("updated_at")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
        
        let hot_fields = self.extract_hot_fields(data, schema);
        
        HotRecord {
            version,
            content_hash,
            updated_at,
            hot_fields,
        }
    }
    
    /// Extract configured hot fields from full record
    fn extract_hot_fields(&self, data: &SpookyValue, schema: &TableSchema) -> HotFields {
        if schema.hot_fields.is_empty() {
            return HotFields::None;
        }
        
        let mut fields = [(0u64, HotFieldValue::Null); 5];
        let mut count = 0;
        
        for (i, config) in schema.hot_fields.iter().take(5).enumerate() {
            let hash = hash_field_name(&config.name);
            let value = data.get(&config.name)
                .map(|v| self.to_hot_field_value(v, config.max_size))
                .unwrap_or(HotFieldValue::Null);
            
            fields[i] = (hash, value);
            count += 1;
        }
        
        HotFields::Generic {
            fields,
            field_count: count,
        }
    }
    
    /// Convert SpookyValue to HotFieldValue
    fn to_hot_field_value(&self, v: &SpookyValue, max_size: usize) -> HotFieldValue {
        match v {
            SpookyValue::Null => HotFieldValue::Null,
            SpookyValue::Bool(b) => HotFieldValue::Bool(*b),
            SpookyValue::Number(n) => {
                if let Some(i) = n.as_i64() {
                    HotFieldValue::Int(i)
                } else {
                    HotFieldValue::Float(n.as_f64().unwrap_or(0.0))
                }
            }
            SpookyValue::String(s) => {
                if s.len() <= max_size {
                    HotFieldValue::String(SmolStr::new(s))
                } else {
                    HotFieldValue::InCold  // Too large, reference cold storage
                }
            }
            // Objects and arrays always go to cold
            SpookyValue::Object(_) | SpookyValue::Array(_) => HotFieldValue::InCold,
        }
    }
    
    /// Compute content hash based on schema configuration
    fn compute_content_hash(&self, data: &SpookyValue, schema: &TableSchema) -> u64 {
        use std::hash::{Hash, Hasher};
        use std::collections::hash_map::DefaultHasher;
        
        let mut hasher = DefaultHasher::new();
        
        match &schema.hash_fields {
            HashFieldsConfig::All => {
                // Hash canonical JSON representation
                let json = serde_json::to_string(data).unwrap_or_default();
                json.hash(&mut hasher);
            }
            HashFieldsConfig::Only(fields) => {
                for field in fields {
                    if let Some(v) = data.get(field.as_str()) {
                        field.hash(&mut hasher);
                        serde_json::to_string(v).unwrap_or_default().hash(&mut hasher);
                    }
                }
            }
            HashFieldsConfig::Except(excluded) => {
                if let SpookyValue::Object(map) = data {
                    for (k, v) in map {
                        if !excluded.iter().any(|e| e == k) {
                            k.hash(&mut hasher);
                            serde_json::to_string(v).unwrap_or_default().hash(&mut hasher);
                        }
                    }
                }
            }
        }
        
        hasher.finish()
    }
    
    /// Convert hot record to partial SpookyValue with only requested fields
    fn hot_to_partial_spooky(&self, hot: &HotRecord, fields: &[&str]) -> SpookyValue {
        let mut map = serde_json::Map::new();
        
        for field in fields {
            let value = match *field {
                "_spooky_version" => Some(SpookyValue::Number(hot.version.into())),
                "updated_at" => Some(SpookyValue::Number(hot.updated_at.into())),
                _ => hot.get_hot_field(field).and_then(|hf| hf.to_spooky_value()),
            };
            
            if let Some(v) = value {
                map.insert(field.to_string(), v);
            }
        }
        
        SpookyValue::Object(map)
    }
}

impl HotFieldValue {
    pub fn to_spooky_value(&self) -> Option<SpookyValue> {
        match self {
            HotFieldValue::Null => Some(SpookyValue::Null),
            HotFieldValue::Bool(b) => Some(SpookyValue::Bool(*b)),
            HotFieldValue::Int(i) => Some(SpookyValue::Number((*i).into())),
            HotFieldValue::Float(f) => Some(SpookyValue::Number(
                serde_json::Number::from_f64(*f).unwrap_or(serde_json::Number::from(0))
            )),
            HotFieldValue::String(s) => Some(SpookyValue::String(s.to_string())),
            HotFieldValue::InCold => None,  // Caller must fetch from cold
        }
    }
}
```

##### Query Optimization with Hot/Cold

```rust
impl View {
    /// Optimized record fetching based on query analysis
    pub fn fetch_records_optimized(
        &self,
        storage: &HotColdStorage,
        table: &str,
        ids: &[SmolStr],
    ) -> Vec<SpookyValue> {
        // Analyze which fields this view actually needs
        let needed_fields = self.plan.root.referenced_fields(table);
        
        // Check if hot storage is sufficient
        let schema = storage.schemas.get(table);
        let can_use_hot = needed_fields.iter().all(|f| {
            f == "id" || 
            f == "_spooky_version" ||
            schema.map(|s| s.hot_fields.iter().any(|hf| &hf.name == f))
                .unwrap_or(false)
        });
        
        if can_use_hot {
            // Fast path: hot storage only
            tracing::debug!(
                target: "ssp::storage",
                table = %table,
                records = ids.len(),
                "Using hot storage path"
            );
            
            ids.iter()
                .filter_map(|id| storage.get_fields(table, id, &needed_fields))
                .collect()
        } else {
            // Slow path: need cold storage
            tracing::debug!(
                target: "ssp::storage", 
                table = %table,
                records = ids.len(),
                needed_fields = ?needed_fields,
                "Falling back to cold storage"
            );
            
            ids.iter()
                .filter_map(|id| storage.get_cold(table, id))
                .collect()
        }
    }
}

impl Operator {
    /// Analyze which fields are actually referenced by this operator
    pub fn referenced_fields(&self, table: &str) -> Vec<&str> {
        let mut fields = Vec::new();
        self.collect_referenced_fields(table, &mut fields);
        fields
    }
    
    fn collect_referenced_fields<'a>(&'a self, table: &str, fields: &mut Vec<&'a str>) {
        match self {
            Operator::Scan { table: t, .. } if t == table => {
                // Scan needs all fields unless there's a projection
                fields.push("*");
            }
            Operator::Filter { predicate, input, .. } => {
                // Collect fields used in predicate
                predicate.collect_field_refs(table, fields);
                input.collect_referenced_fields(table, fields);
            }
            Operator::Project { columns, input, .. } => {
                // Only need projected columns
                for col in columns {
                    if col.table.as_deref() == Some(table) || col.table.is_none() {
                        fields.push(&col.name);
                    }
                }
                input.collect_referenced_fields(table, fields);
            }
            Operator::Join { left, right, condition, .. } => {
                condition.collect_field_refs(table, fields);
                left.collect_referenced_fields(table, fields);
                right.collect_referenced_fields(table, fields);
            }
            // ... handle other operators
            _ => {}
        }
    }
}
```

##### Performance Comparison

```
┌─────────────────────────────────────────────────────────────────────────┐
│                    OPTION D PERFORMANCE ANALYSIS                         │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  Benchmark: 10,000 records, average 1.5KB each                          │
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ Operation: Filter by status (hot field)                         │    │
│  │ Query: SELECT * FROM users WHERE status = 'active'              │    │
│  │ Records matching: 3,000                                         │    │
│  ├─────────────────────────────────────────────────────────────────┤    │
│  │                                                                  │    │
│  │ Option A (JSON):     Read 15MB, parse 15MB      │ ~750ms        │    │
│  │ Option B (per-field): Read 300KB (status only)  │ ~15ms         │    │
│  │ Option C (Hybrid):   Read 15MB, parse 300KB     │ ~200ms        │    │
│  │ Option D (Hot/Cold): Read 1MB (hot), filter     │ ~25ms      ✅ │    │
│  │                      Then read 4.5MB (cold)     │ +~100ms       │    │
│  │                      Total for full records     │ ~125ms        │    │
│  │                                                                  │    │
│  │ If view only needs id, status (hot fields):     │ ~25ms      ✅ │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ Operation: Delta propagation (version check)                    │    │
│  │ Check: Which records changed since last sync?                   │    │
│  │ Records to check: 10,000                                        │    │
│  ├─────────────────────────────────────────────────────────────────┤    │
│  │                                                                  │    │
│  │ Option A (JSON):     Read 15MB, parse versions  │ ~500ms        │    │
│  │ Option C (Hybrid):   Read 15MB, scan headers    │ ~150ms        │    │
│  │ Option D (Hot/Cold): Read 1MB (hot only)        │ ~20ms      ✅ │    │
│  │                                                                  │    │
│  │ Hot table has version + content_hash = instant delta detection  │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ Operation: Full table scan for view computation                 │    │
│  │ Query: SELECT id, name, email, bio FROM users                   │    │
│  │ Fields: 2 hot (id, name), 2 cold (email, bio)                  │    │
│  ├─────────────────────────────────────────────────────────────────┤    │
│  │                                                                  │    │
│  │ Option A (JSON):     Read 15MB                  │ ~500ms        │    │
│  │ Option C (Hybrid):   Read 15MB                  │ ~200ms        │    │
│  │ Option D (Hot/Cold): Must read cold (mixed)     │ ~500ms        │    │
│  │                                                                  │    │
│  │ Note: Option D doesn't help when you need cold fields           │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

##### Pros and Cons Summary

```
┌─────────────────────────────────────────────────────────────────────────┐
│                     OPTION D: PROS AND CONS                              │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  ✅ PROS                                                                 │
│  ───────────────────────────────────────────────────────────────────    │
│  • Minimal I/O for common DBSP operations (70-85% hot table hits)       │
│  • Version/hash checks are O(100 bytes) not O(2KB)                      │
│  • Hot table fits in cache better (10x smaller working set)             │
│  • Filter queries on hot fields are dramatically faster                  │
│  • Configurable per-table based on access patterns                       │
│  • Cold storage can use simple JSON (human readable, debuggable)        │
│  • Hot storage uses rkyv (zero-copy, compact)                           │
│  • Write overhead is acceptable (2 small writes vs 1 large write)       │
│  • Natural fit for DBSP change detection (version + hash in hot)        │
│  • Graceful degradation: cold path still works for complex queries      │
│                                                                          │
│  ❌ CONS                                                                 │
│  ───────────────────────────────────────────────────────────────────    │
│  • Two tables per logical record (more complexity)                       │
│  • Must keep hot and cold in sync (transaction overhead)                │
│  • Schema configuration required (which fields are hot?)                │
│  • Queries needing cold fields don't benefit                            │
│  • Storage overhead: ~100 bytes per record for hot table                │
│  • Must analyze queries to determine if hot path is usable              │
│  • Hot field changes require migration                                   │
│  • More complex debugging (data in two places)                          │
│                                                                          │
│  ⚠️ TRADE-OFFS                                                          │
│  ───────────────────────────────────────────────────────────────────    │
│  • Write amplification: 2x writes, but both are smaller                 │
│  • Complexity vs performance: significant gains for DBSP workloads      │
│  • Configuration burden: must understand access patterns                 │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

##### When to Use Option D

```
┌─────────────────────────────────────────────────────────────────────────┐
│                    OPTION D DECISION GUIDE                               │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  USE Option D when:                                                      │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ ✓ Records are large (>500 bytes average)                        │    │
│  │ ✓ Most operations need only a few fields                        │    │
│  │ ✓ You have predictable "hot" fields (status, timestamps, IDs)   │    │
│  │ ✓ Delta detection (version/hash) is frequent                    │    │
│  │ ✓ Filter queries dominate your workload                         │    │
│  │ ✓ Dataset doesn't fit comfortably in page cache                 │    │
│  │ ✓ You can tolerate slightly more complex write path             │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
│  DON'T USE Option D when:                                               │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ ✗ Records are small (<300 bytes) - overhead not worth it        │    │
│  │ ✗ Most queries need all fields anyway                           │    │
│  │ ✗ No predictable hot fields (random access patterns)            │    │
│  │ ✗ Simplicity is more important than performance                 │    │
│  │ ✗ Write throughput is critical (2x write overhead hurts)        │    │
│  │ ✗ Dataset fits easily in memory (page cache handles it)         │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
│  SSP RECOMMENDATION:                                                     │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ Option D is IDEAL for SSP because:                              │    │
│  │ • DBSP constantly checks versions and hashes (hot path)         │    │
│  │ • ZSet operations need only record existence (hot path)         │    │
│  │ • Filter predicates often use status/category (configurable)    │    │
│  │ • Full records only needed for final view output                │    │
│  │ • Local-first sync benefits from compact delta checks           │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

##### Migration Path: Adding Option D

```
┌─────────────────────────────────────────────────────────────────────────┐
│                    MIGRATION TO OPTION D                                 │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  Phase 1: Implement Storage Abstraction (Week 1)                        │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ • Define Storage trait with get_hot / get_cold / get_fields     │    │
│  │ • Implement MemoryStorage (current behavior, both return same)  │    │
│  │ • Refactor Circuit to use Arc<dyn Storage>                      │    │
│  │ • All tests pass, no behavior change                            │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                              │                                           │
│                              ▼                                           │
│  Phase 2: Add redb with Single Table (Week 2)                           │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ • Implement RedbStorage with just cold table (Option A/C)       │    │
│  │ • Benchmark baseline performance                                 │    │
│  │ • Identify actual bottlenecks with production data              │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                              │                                           │
│                              ▼                                           │
│  Phase 3: Add Hot Table (Week 3)                                        │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ • Add record_hot table with HotRecord structure                  │    │
│  │ • Implement dual-write in put_record                            │    │
│  │ • Add get_hot method                                            │    │
│  │ • Feature flag: --hot-cold-split                                │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                              │                                           │
│                              ▼                                           │
│  Phase 4: Query Optimization (Week 4)                                   │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ • Implement referenced_fields() analysis in Operator            │    │
│  │ • Add hot path routing in View::fetch_records_optimized         │    │
│  │ • Configure TableSchema for each table                          │    │
│  │ • Benchmark improvement                                          │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                              │                                           │
│                              ▼                                           │
│  Phase 5: Tune and Optimize (Ongoing)                                   │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │ • Monitor hot table hit rate                                     │    │
│  │ • Adjust hot field configurations based on real usage           │    │
│  │ • Add rkyv to hot table for zero-copy reads                     │    │
│  │ • Consider rkyv for view_cache                                  │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

### When rkyv Makes Sense

| Scenario | Use rkyv? | Why |
|----------|-----------|-----|
| Large records (>1KB) | ✅ Yes | Zero-copy avoids allocation |
| Hot path reads | ✅ Yes | Direct memory access |
| Field projection queries | ✅ Yes | Read field without deserializing others |
| Small records (<256 bytes) | ❌ No | Overhead not worth it |
| WASM target | ⚠️ Maybe | rkyv works but less benefit |
| Frequent writes | ⚠️ Maybe | Serialization cost on write |
| View cache storage | ✅ Yes | Large arrays benefit most |

**rkyv Sweet Spot:** Records > 512 bytes where you frequently access individual fields without needing the whole object.

### Implementation Strategy

#### Phase 1: Abstract Storage Trait

```rust
pub trait Storage: Send + Sync {
    // Record operations
    fn get_record(&self, table: &str, id: &str) -> Option<SpookyValue>;
    fn get_field(&self, table: &str, id: &str, field: &str) -> Option<SpookyValue>;
    fn put_record(&self, table: &str, id: &str, data: &SpookyValue) -> Result<()>;
    fn delete_record(&self, table: &str, id: &str) -> Result<()>;
    
    // ZSet operations  
    fn get_weight(&self, key: &str) -> i64;
    fn update_weight(&self, key: &str, delta: i64) -> Result<i64>;
    
    // Batch operations
    fn transaction<F, R>(&self, f: F) -> Result<R>
    where F: FnOnce(&mut dyn StorageTx) -> Result<R>;
    
    // Iteration (for view evaluation)
    fn scan_table(&self, table: &str) -> Box<dyn Iterator<Item = (SmolStr, SpookyValue)>>;
    fn scan_zset_prefix(&self, prefix: &str) -> Box<dyn Iterator<Item = (SmolStr, i64)>>;
}

pub trait StorageTx {
    fn put_record(&mut self, table: &str, id: &str, data: &SpookyValue);
    fn delete_record(&mut self, table: &str, id: &str);
    fn update_weight(&mut self, key: &str, delta: i64);
}
```

#### Phase 2: In-Memory Implementation (Current Behavior)

```rust
pub struct MemoryStorage {
    records: RwLock<FastMap<String, FastMap<SmolStr, SpookyValue>>>,
    zsets: RwLock<FastMap<SmolStr, i64>>,
}

impl Storage for MemoryStorage {
    fn get_record(&self, table: &str, id: &str) -> Option<SpookyValue> {
        self.records.read().get(table)?.get(id).cloned()
    }
    // ... etc
}
```

#### Phase 3: redb Implementation

```rust
use redb::{Database, ReadableTable, TableDefinition};

const RECORDS: TableDefinition<&str, &[u8]> = TableDefinition::new("records");
const ZSETS: TableDefinition<&str, i64> = TableDefinition::new("zsets");
const VIEW_CACHE: TableDefinition<&str, &[u8]> = TableDefinition::new("view_cache");

pub struct RedbStorage {
    db: Database,
}

impl RedbStorage {
    pub fn open(path: &Path) -> Result<Self> {
        let db = Database::create(path)?;
        Ok(Self { db })
    }
}

impl Storage for RedbStorage {
    fn get_record(&self, table: &str, id: &str) -> Option<SpookyValue> {
        let key = format!("{}:{}", table, id);
        let read_txn = self.db.begin_read().ok()?;
        let table = read_txn.open_table(RECORDS).ok()?;
        let bytes = table.get(key.as_str()).ok()??;
        
        // Deserialize (JSON or rkyv)
        serde_json::from_slice(bytes.value()).ok()
    }
    
    fn transaction<F, R>(&self, f: F) -> Result<R>
    where F: FnOnce(&mut dyn StorageTx) -> Result<R>
    {
        let write_txn = self.db.begin_write()?;
        let mut tx = RedbTx { txn: write_txn };
        let result = f(&mut tx)?;
        tx.txn.commit()?;
        Ok(result)
    }
}
```

#### Phase 4: rkyv Integration (Optional)

```rust
use rkyv::{Archive, Deserialize, Serialize, archived_root, to_bytes};

#[derive(Archive, Deserialize, Serialize)]
pub struct ArchivedRecord {
    version: i64,
    // Field offsets for zero-copy access
    field_offsets: Vec<(u64, u32, u32)>,  // (name_hash, offset, len)
    data: Vec<u8>,
}

impl ArchivedRecord {
    /// Zero-copy field access - returns slice into archived data
    pub fn get_field_bytes(&self, field_hash: u64) -> Option<&[u8]> {
        let (_, offset, len) = self.field_offsets
            .iter()
            .find(|(h, _, _)| *h == field_hash)?;
        Some(&self.data[*offset as usize..(*offset + *len) as usize])
    }
}

pub struct RkyvRedbStorage {
    db: Database,
}

impl Storage for RkyvRedbStorage {
    fn get_field(&self, table: &str, id: &str, field: &str) -> Option<SpookyValue> {
        let key = format!("{}:{}", table, id);
        let read_txn = self.db.begin_read().ok()?;
        let tbl = read_txn.open_table(RECORDS).ok()?;
        let bytes = tbl.get(key.as_str()).ok()??;
        
        // Zero-copy access!
        let archived = unsafe { archived_root::<ArchivedRecord>(bytes.value()) };
        let field_hash = hash_field_name(field);
        let field_bytes = archived.get_field_bytes(field_hash)?;
        
        // Only deserialize the one field
        rkyv::from_bytes(field_bytes).ok()
    }
}
```

### Performance Characteristics

| Operation | In-Memory | redb (JSON) | redb (rkyv) |
|-----------|-----------|-------------|-------------|
| Get full record | O(1) ~50ns | O(1) ~5µs | O(1) ~2µs |
| Get single field | O(1) ~30ns | O(1) ~5µs + deser | O(1) ~500ns |
| Put record | O(1) ~100ns | O(1) ~50µs | O(1) ~30µs |
| Scan table (1K records) | O(n) ~50µs | O(n) ~5ms | O(n) ~2ms |
| Transaction (10 ops) | N/A | ~100µs | ~80µs |

**Key insight:** redb is ~100x slower than in-memory for individual ops, but:
1. Memory stays bounded
2. Crash recovery is free
3. Multiple circuits share one copy
4. Large datasets become possible

### Impact on Each Optimization

#### 1. Parallel Views

**Before:** Each view reads from `&self.db` (in-memory)
**After:** Each view reads from `Arc<dyn Storage>` 

```rust
// Old
pub fn process_batch(&mut self, batch_deltas: &BatchDeltas, db: &Database) -> Option<ViewUpdate>

// New  
pub fn process_batch(&mut self, batch_deltas: &BatchDeltas, storage: &Arc<dyn Storage>) -> Option<ViewUpdate>
```

**Impact:** 
- ✅ Parallel reads are safe (redb supports concurrent readers)
- ⚠️ Parallel writes need coordination (single writer in redb)
- ✅ No change to Rayon parallelism for view processing

#### 2. Batch Ingestion

**Before:** Mutations directly modify HashMap
**After:** Mutations go through transaction

```rust
impl Circuit {
    pub fn ingest_batch(&mut self, entries: Vec<BatchEntry>) -> Vec<ViewUpdate> {
        // Must wrap in transaction for atomicity
        self.storage.transaction(|tx| {
            for entry in &entries {
                match entry.op {
                    Operation::Create | Operation::Update => {
                        tx.put_record(&entry.table, &entry.id, &entry.data);
                    }
                    Operation::Delete => {
                        tx.delete_record(&entry.table, &entry.id);
                    }
                }
                tx.update_weight(&make_zset_key(&entry.table, &entry.id), entry.op.weight());
            }
            Ok(())
        })?;
        
        // Then propagate deltas (reads don't need transaction)
        self.propagate_deltas(...)
    }
}
```

**Impact:**
- ✅ Batch operations become truly atomic
- ✅ Crash mid-batch = rollback (safe)
- ⚠️ Transaction overhead (~50µs per batch)
- ⚠️ Write lock contention if multiple circuits write simultaneously

#### 3. View Caching

**Before:** `view.cache: Vec<SpookyValue>` in memory per view
**After:** Cache stored in persistent storage, survives restarts

```rust
impl View {
    pub fn save_cache(&self, storage: &dyn Storage) {
        let cache_data = CacheData {
            results: &self.cache,
            hash: &self.last_hash,
            params_hash: self.params_hash,
        };
        storage.put_view_cache(&self.plan.id, &cache_data);
    }
    
    pub fn load_cache(&mut self, storage: &dyn Storage) -> bool {
        if let Some(cache_data) = storage.get_view_cache(&self.plan.id) {
            if cache_data.params_hash == self.params_hash {
                self.cache = cache_data.results;
                self.last_hash = cache_data.hash;
                return true;  // Cache hit!
            }
        }
        false
    }
}
```

**Impact:**
- ✅ Restart doesn't require full recomputation
- ✅ Invalidation is clearer (delete cache key)
- ⚠️ Cache serialization/deserialization cost
- ✅ rkyv shines here - large cache arrays benefit from zero-copy

#### 4. Circuit Sharding

**Before:** Each circuit has full copy of data
**After:** Circuits share storage, only maintain their own views/ZSets

```rust
pub struct ShardedCircuitGroup {
    storage: Arc<dyn Storage>,  // Shared!
    circuits: Vec<Circuit>,
}

pub struct Circuit {
    storage: Arc<dyn Storage>,  // Reference to shared storage
    views: Vec<View>,           // Circuit-specific
    zsets: FastMap<SmolStr, i64>,  // Could also be in storage with prefix
    dependency_list: FastMap<TableName, DependencyList>,
}
```

**Impact:**
- ✅ **Data duplication eliminated** - biggest win!
- ✅ Cross-circuit queries possible (scatter-gather from shared storage)
- ✅ Memory usage = O(records) not O(records × circuits)
- ⚠️ Write coordination needed (which circuit "owns" writes?)

**Sharding becomes much more attractive with shared storage:**

```
Before (in-memory):
┌─────────────────┐  ┌─────────────────┐
│ Circuit A       │  │ Circuit B       │
│ users: 100MB    │  │ users: 100MB    │  ← Duplicated!
│ threads: 200MB  │  │ threads: 200MB  │  ← Duplicated!
│ views: 50MB     │  │ views: 30MB     │
│ Total: 350MB    │  │ Total: 330MB    │
└─────────────────┘  └─────────────────┘
                     Total: 680MB

After (shared storage):
┌─────────────────────────────────────┐
│ Shared Storage                       │
│ users: 100MB                         │
│ threads: 200MB                       │
│ Total records: 300MB                 │
└─────────────────────────────────────┘
       ▲                    ▲
┌──────┴────────┐  ┌───────┴───────┐
│ Circuit A     │  │ Circuit B     │
│ views: 50MB   │  │ views: 30MB   │
│ zsets: 5MB    │  │ zsets: 5MB    │
└───────────────┘  └───────────────┘
                   Total: 390MB (43% reduction)
```

#### 5. Message Queue

**Impact:** Unchanged - still probably not needed.

If you do add a message queue for durability, the persistent storage layer already provides crash recovery, making the queue even less necessary.

---

## 7. Smart Key-Value Design Patterns

### Pattern 1: Composite Keys with Separators

```rust
// Use consistent separator that won't appear in IDs
const SEP: char = '\x1F';  // ASCII Unit Separator

fn record_key(table: &str, id: &str) -> String {
    format!("{}{}{}", table, SEP, id)
}

fn field_key(table: &str, id: &str, field: &str) -> String {
    format!("{}{}{}{}{}", table, SEP, id, SEP, field)
}

fn zset_key(table: &str, id: &str) -> String {
    format!("z{}{}{}{}", SEP, table, SEP, id)
}

fn view_cache_key(view_id: &str) -> String {
    format!("vc{}{}", SEP, view_id)
}
```

**Why:** Enables prefix scans, avoids collisions, sorts correctly.

### Pattern 2: Prefix Scans for Table Iteration

```rust
impl RedbStorage {
    fn scan_table(&self, table: &str) -> impl Iterator<Item = (SmolStr, SpookyValue)> {
        let prefix = format!("{}\x1F", table);
        let read_txn = self.db.begin_read().unwrap();
        let tbl = read_txn.open_table(RECORDS).unwrap();
        
        tbl.range(prefix.as_str()..)
            .take_while(|r| r.as_ref().map(|(k, _)| k.value().starts_with(&prefix)).unwrap_or(false))
            .filter_map(|r| {
                let (k, v) = r.ok()?;
                let id = k.value().strip_prefix(&prefix)?;
                let value = deserialize(v.value())?;
                Some((SmolStr::new(id), value))
            })
    }
}
```

### Pattern 3: Field Projection Without Full Deserialize

```rust
/// Record format with field index header
/// 
/// Layout:
/// [4 bytes: field_count]
/// [field_count × 16 bytes: (field_hash: u64, offset: u32, len: u32)]
/// [variable: field data concatenated]

pub struct FieldProjector<'a> {
    data: &'a [u8],
    field_count: u32,
}

impl<'a> FieldProjector<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        let field_count = u32::from_le_bytes(data[0..4].try_into().unwrap());
        Self { data, field_count }
    }
    
    pub fn get_field(&self, field_name: &str) -> Option<&'a [u8]> {
        let target_hash = hash_field_name(field_name);
        let header_size = 4 + (self.field_count as usize * 16);
        
        for i in 0..self.field_count as usize {
            let entry_offset = 4 + (i * 16);
            let hash = u64::from_le_bytes(self.data[entry_offset..entry_offset+8].try_into().unwrap());
            
            if hash == target_hash {
                let offset = u32::from_le_bytes(self.data[entry_offset+8..entry_offset+12].try_into().unwrap());
                let len = u32::from_le_bytes(self.data[entry_offset+12..entry_offset+16].try_into().unwrap());
                let start = header_size + offset as usize;
                return Some(&self.data[start..start + len as usize]);
            }
        }
        None
    }
    
    /// Get multiple fields efficiently (single pass through header)
    pub fn get_fields(&self, fields: &[&str]) -> Vec<Option<&'a [u8]>> {
        let target_hashes: Vec<_> = fields.iter().map(|f| hash_field_name(f)).collect();
        let mut results = vec![None; fields.len()];
        let header_size = 4 + (self.field_count as usize * 16);
        
        for i in 0..self.field_count as usize {
            let entry_offset = 4 + (i * 16);
            let hash = u64::from_le_bytes(self.data[entry_offset..entry_offset+8].try_into().unwrap());
            
            if let Some(idx) = target_hashes.iter().position(|&h| h == hash) {
                let offset = u32::from_le_bytes(self.data[entry_offset+8..entry_offset+12].try_into().unwrap());
                let len = u32::from_le_bytes(self.data[entry_offset+12..entry_offset+16].try_into().unwrap());
                let start = header_size + offset as usize;
                results[idx] = Some(&self.data[start..start + len as usize]);
            }
        }
        results
    }
}
```

### Pattern 4: Secondary Indexes

```rust
const INDEXES: TableDefinition<&str, ()> = TableDefinition::new("indexes");

impl RedbStorage {
    /// Create index entry
    fn index_field(&self, tx: &mut WriteTransaction, table: &str, id: &str, field: &str, value: &SpookyValue) {
        // Index key format: {table}:idx:{field}:{value_prefix}:{id}
        let value_str = value_to_indexable_string(value);
        let key = format!("{}:idx:{}:{}:{}", table, field, value_str, id);
        
        let mut idx_table = tx.open_table(INDEXES).unwrap();
        idx_table.insert(key.as_str(), ()).unwrap();
    }
    
    /// Query by indexed field
    fn query_by_field(&self, table: &str, field: &str, value: &SpookyValue) -> Vec<SmolStr> {
        let value_str = value_to_indexable_string(value);
        let prefix = format!("{}:idx:{}:{}:", table, field, value_str);
        
        let read_txn = self.db.begin_read().unwrap();
        let idx_table = read_txn.open_table(INDEXES).unwrap();
        
        idx_table.range(prefix.as_str()..)
            .take_while(|r| r.as_ref().map(|(k, _)| k.value().starts_with(&prefix)).unwrap_or(false))
            .filter_map(|r| {
                let (k, _) = r.ok()?;
                let id = k.value().rsplit(':').next()?;
                Some(SmolStr::new(id))
            })
            .collect()
    }
}

fn value_to_indexable_string(v: &SpookyValue) -> String {
    match v {
        SpookyValue::String(s) => s.chars().take(64).collect(),  // Truncate for index
        SpookyValue::Number(n) => format!("{:020.6}", n),  // Fixed width for sorting
        SpookyValue::Bool(b) => if *b { "1" } else { "0" }.to_string(),
        _ => "".to_string(),
    }
}
```

---

## 8. When to Use rkyv vs JSON

### Decision Matrix

```
                        ┌─────────────────────────────────────┐
                        │         Record Size                  │
                        ├────────────┬────────────┬───────────┤
                        │  <256B     │  256B-4KB  │   >4KB    │
┌───────────────────────┼────────────┼────────────┼───────────┤
│ Read whole record     │   JSON     │   Either   │   rkyv    │
│ Read 1-2 fields       │   JSON     │   rkyv     │   rkyv    │
│ Frequent updates      │   JSON     │   JSON     │   Either  │
│ Archival/cold storage │   Either   │   rkyv     │   rkyv    │
│ Debug/inspect needed  │   JSON     │   JSON     │   JSON    │
│ WASM target           │   JSON     │   JSON     │   JSON*   │
└───────────────────────┴────────────┴────────────┴───────────┘

* rkyv works in WASM but memory mapping benefits are lost
```

### Concrete Recommendations for SSP

| Data Type | Format | Reasoning |
|-----------|--------|-----------|
| User records (~200B) | JSON | Small, frequently updated |
| Thread records (~500B) | JSON or rkyv | Medium size, moderate updates |
| Message content (1KB+) | rkyv | Large, benefits from field projection |
| View cache (variable) | rkyv | Large arrays, read-heavy |
| ZSet weights | Raw i64 | Trivial, no serialization needed |
| Metadata/config | JSON | Human readable, rarely accessed |

### rkyv Implementation Notes

```rust
// Cargo.toml
[dependencies]
rkyv = { version = "0.7", features = ["validation", "strict"] }

// For SpookyValue, you'll need custom Archive impl or wrapper
#[derive(Archive, Deserialize, Serialize)]
#[archive(check_bytes)]  // Enable validation
pub struct ArchivedSpookyRecord {
    pub version: i64,
    pub fields: ArchivedVec<ArchivedField>,
}

#[derive(Archive, Deserialize, Serialize)]
pub struct ArchivedField {
    pub name_hash: u64,
    pub value: ArchivedFieldValue,
}

#[derive(Archive, Deserialize, Serialize)]
pub enum ArchivedFieldValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(ArchivedString),
    // Note: Nested objects/arrays get complex with rkyv
}
```

**rkyv Gotchas:**
1. Nested structures (JSON objects in objects) are painful
2. Dynamic typing (SpookyValue) doesn't map cleanly to rkyv's static types
3. Schema evolution is harder than JSON
4. `unsafe` required for zero-copy access (use `check_bytes` feature)

**Pragmatic approach:** Use rkyv for view cache (homogeneous arrays), keep JSON for records (dynamic schema).

---

## 9. Updated Implementation Roadmap

### Phase 1: Storage Abstraction (Week 1-2)

- [ ] Define `Storage` trait
- [ ] Implement `MemoryStorage` matching current behavior
- [ ] Refactor `Circuit` to use `Arc<dyn Storage>`
- [ ] All tests pass with no behavior change

### Phase 2: redb Integration (Week 3-4)

- [ ] Add redb dependency
- [ ] Implement `RedbStorage` with JSON serialization
- [ ] Add storage selection in circuit creation
- [ ] Benchmark: memory vs redb performance
- [ ] Add transaction support for batch operations

### Phase 3: View Cache Persistence (Week 5)

- [ ] Add `view_cache` table to storage
- [ ] Implement cache save/load in View
- [ ] Add cache invalidation on schema change
- [ ] Test restart recovery

### Phase 4: rkyv Optimization (Week 6+, Optional)

- [ ] Evaluate real-world record sizes
- [ ] If >1KB average: implement rkyv for records
- [ ] Implement rkyv for view cache (high value)
- [ ] Benchmark improvement

### Phase 5: Circuit Sharding with Shared Storage (When Needed)

- [ ] Refactor to shared storage model
- [ ] Implement write coordination
- [ ] Test multi-circuit scenarios

---

## 10. Summary

### Key Architectural Decisions

| Decision | Recommendation | Rationale |
|----------|---------------|-----------|
| Storage backend | redb | Embedded, fast, ACID, no server |
| Record serialization | JSON (start), rkyv (optimize later) | Flexibility first, speed second |
| Field-level access | Hybrid with offset header | Best of both worlds |
| View cache storage | Persistent in redb | Survives restarts |
| Circuit sharding | Shared storage makes it viable | Eliminates data duplication |

### Updated Priority Matrix

| Optimization | Effort | Impact | Priority | Storage Dependency |
|-------------|--------|--------|----------|-------------------|
| **Storage Layer** | Medium | High 🚀 | **DO SOON** | Foundation |
| Parallel Views | Low ✅ | High 🚀 | **DO NOW** | Works with both |
| Batch Ingestion | Low ✅ | Medium 📈 | **DO NOW** | Better with transactions |
| View Caching | Medium | Medium 📈 | **After storage** | Needs persistence |
| Circuit Sharding | Medium* | High 🚀 | **After storage** | *Much easier with shared storage |
| Message Queue | High ⚠️ | Medium 📈 | **Never** | Storage provides durability |

### The Big Picture

```
Current State:
┌─────────────────────────────────────┐
│ Circuit                              │
│ ┌─────────────────────────────────┐ │
│ │ In-Memory Database (HashMap)    │ │  ← Memory-bound
│ │ - records: FastMap              │ │  ← Lost on crash
│ │ - zsets: FastMap                │ │  ← Duplicated if sharded
│ └─────────────────────────────────┘ │
│ ┌─────────────────────────────────┐ │
│ │ Views (in-memory cache)         │ │  ← Lost on crash
│ └─────────────────────────────────┘ │
└─────────────────────────────────────┘

Future State:
┌─────────────────────────────────────┐
│ Storage Layer (redb)                │  ← Persistent
│ ┌─────────────────────────────────┐ │  ← Shared across circuits
│ │ records table (JSON/rkyv)       │ │  ← Crash-safe
│ │ zsets table                     │ │
│ │ view_cache table (rkyv)         │ │  ← Fast restart
│ └─────────────────────────────────┘ │
└─────────────────────────────────────┘
        ▲           ▲           ▲
        │           │           │
┌───────┴───┐ ┌─────┴─────┐ ┌───┴───────┐
│ Circuit A │ │ Circuit B │ │ Circuit C │
│ (views)   │ │ (views)   │ │ (views)   │
└───────────┘ └───────────┘ └───────────┘
```

The persistent storage layer is the foundation that makes all other optimizations more effective and circuit sharding actually practical.
