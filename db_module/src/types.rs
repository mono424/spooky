use super::spooky_record::{SpookyReadable, SpookyRecord};

// ─── Type Tags ──────────────────────────────────────────────────────────────
pub const TAG_NULL: u8 = 0;
pub const TAG_BOOL: u8 = 1;
pub const TAG_I64: u8 = 2;
pub const TAG_F64: u8 = 3;
pub const TAG_STR: u8 = 4;
pub const TAG_NESTED_CBOR: u8 = 5; // Array or Object
pub const TAG_U64: u8 = 6; // Extension

// ─── Binary Layout ──────────────────────────────────────────────────────────
//
//  ┌──────────────────────────────────────────────┐
//  │ Header (20 bytes)                            │
//  │   field_count: u32 (LE)                      │
//  │   _reserved: [u8; 16]                        │
//  ├──────────────────────────────────────────────┤
//  │ Index (20 bytes × field_count)               │
//  │   name_hash:   u64 (LE)    ← SORTED by hash │
//  │   data_offset: u32 (LE)                      │
//  │   data_length: u32 (LE)                      │
//  │   type_tag:    u8                            │
//  │   _padding:    [u8; 3]                       │
//  ├──────────────────────────────────────────────┤
//  │ Data (variable)                              │
//  │   field values packed sequentially           │
//  └──────────────────────────────────────────────┘

pub const HEADER_SIZE: usize = 20; // 4 + 16
pub const INDEX_ENTRY_SIZE: usize = 20; // 8 + 4 + 4 + 1 + 3

// ─── FieldSlot (Cached Field Position) ─────────────────────────────────────

/// Cached field position for O(1) access.
///
/// Holds everything needed to read/write a field directly without hashing
/// or searching. Valid only while `generation` matches the record's generation.
/// Staleness is checked via debug assertions.
/// A parsed index entry from the binary header. Internal only.
#[derive(Debug, Clone, Copy)]
pub struct IndexEntry {
    pub name_hash: u64,
    pub data_offset: usize,
    pub data_len: usize, // data_length → data_len (matches Rust convention: .len())
    pub type_tag: u8,
}

/// A raw, zero-copy reference to a field's bytes. No deserialization.
#[derive(Debug, Clone, Copy)]
pub struct FieldRef<'a> {
    pub name_hash: u64,
    pub type_tag: u8,
    pub data: &'a [u8],
}

/// Cached field position for O(1) repeat access. Invalidated by mutation.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct FieldSlot {
    pub(crate) index_pos: usize,
    pub(crate) data_offset: usize,
    pub(crate) data_len: usize, // consistent with IndexEntry
    pub(crate) type_tag: u8,
    pub(crate) generation: usize,
}

// ─── Iterator ───────────────────────────────────────────────────────────────

pub struct FieldIter<'a> {
    pub record: SpookyRecord<'a>,
    pub pos: usize,
}

impl<'a> Iterator for FieldIter<'a> {
    type Item = FieldRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.record.field_count {
            return None;
        }
        let entry = self.record.read_index(self.pos)?;
        let data = &self.record.data_buf[entry.data_offset..entry.data_offset + entry.data_len];
        self.pos += 1;
        Some(FieldRef {
            name_hash: entry.name_hash,
            type_tag: entry.type_tag,
            data,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.record.field_count - self.pos;
        (remaining, Some(remaining))
    }
}

impl<'a> ExactSizeIterator for FieldIter<'a> {}
