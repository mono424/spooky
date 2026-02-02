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
