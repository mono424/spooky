use crate::error::RecordError;
use crate::spooky_value::SpookyValue;
use crate::types::*;
use xxhash_rust::xxh64::xxh64;

pub trait SpookyReadable {
    fn data_buf(&self) -> &[u8];
    fn field_count(&self) -> usize;
    /// Iterate over all raw fields (zero-copy)
    fn iter_fields(&self) -> FieldIter<'_>;

    #[inline]
    fn generation(&self) -> usize {
        0
    }

    #[inline]
    fn read_index(&self, i: usize) -> Option<IndexEntry> {
        if i >= self.field_count() {
            return None;
        }
        let buf = self.data_buf();
        let idx = HEADER_SIZE + i * INDEX_ENTRY_SIZE;
        let name_ptr = buf[idx..idx + 8].as_ptr() as *const u64;
        let offset_ptr = buf[idx + 8..idx + 12].as_ptr() as *const u32;
        let length_ptr = buf[idx + 12..idx + 16].as_ptr() as *const u32;

        Some(IndexEntry {
            name_hash: u64::from_le(unsafe { name_ptr.read_unaligned() }),
            data_offset: u32::from_le(unsafe { offset_ptr.read_unaligned() }) as usize,
            data_len: u32::from_le(unsafe { length_ptr.read_unaligned() }) as usize,
            type_tag: buf[idx + 16],
        })
    }

    #[inline]
    fn read_hash(&self, i: usize) -> u64 {
        let buf = self.data_buf();
        let idx = HEADER_SIZE + i * INDEX_ENTRY_SIZE;
        // SAFETY: caller ensures i < field_count, validated at construction
        let ptr = buf[idx..].as_ptr() as *const u64;
        u64::from_le(unsafe { ptr.read_unaligned() })
    }

    #[inline]
    fn linear_hash_search(&self, n: usize, hash: u64) -> Result<(usize, IndexEntry), RecordError> {
        for i in 0..n {
            if self.read_hash(i) == hash {
                return self
                    .read_index(i)
                    .map(|meta| (i, meta))
                    .ok_or(RecordError::InvalidBuffer);
            }
        }
        Err(RecordError::FieldNotFound)
    }

    #[inline]
    fn binary_hash_search(&self, n: usize, hash: u64) -> Result<(usize, IndexEntry), RecordError> {
        // Binary search on sorted hashes
        let mut lo = 0usize;
        let mut hi = n;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let mid_hash = self.read_hash(mid);
            match mid_hash.cmp(&hash) {
                std::cmp::Ordering::Equal => {
                    let meta = self.read_index(mid).ok_or(RecordError::InvalidBuffer)?;
                    return Ok((mid, meta));
                }
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
            }
        }
        Err(RecordError::FieldNotFound)
    }

    /// Find a field by name. Returns (index_position, IndexEntry).
    #[inline]
    fn find_field(&self, name: &str) -> Result<(usize, IndexEntry), RecordError> {
        let hash = xxh64(name.as_bytes(), 0);
        let n = self.field_count();

        if n == 0 {
            return Err(RecordError::FieldNotFound);
        }
        if n <= 4 {
            return self.linear_hash_search(n, hash);
        }
        return self.binary_hash_search(n, hash);
    }

    // ════════════════════════════════════════════════════════════════════════
    // Read access (zero-copy on the mutable buffer)
    // ════════════════════════════════════════════════════════════════════════

    /// Get a string field (zero-copy).
    #[inline]
    fn get_str(&self, name: &str) -> Option<&str> {
        let (_, meta) = self.find_field(name).ok()?;
        if meta.type_tag != TAG_STR {
            return None;
        }
        std::str::from_utf8(&self.data_buf()[meta.data_offset..meta.data_offset + meta.data_len])
            .ok()
    }

    /// Get an i64 field.
    #[inline]
    fn get_i64(&self, name: &str) -> Option<i64> {
        let (_, meta) = self.find_field(name).ok()?;
        if meta.type_tag != TAG_I64 || meta.data_len != 8 {
            return None;
        }
        Some(i64::from_le_bytes(
            self.data_buf()[meta.data_offset..meta.data_offset + 8]
                .try_into()
                .ok()?,
        ))
    }

    /// Get a u64 field.
    #[inline]
    fn get_u64(&self, name: &str) -> Option<u64> {
        let (_, meta) = self.find_field(name).ok()?;
        if meta.type_tag != TAG_U64 || meta.data_len != 8 {
            return None;
        }
        Some(u64::from_le_bytes(
            self.data_buf()[meta.data_offset..meta.data_offset + 8]
                .try_into()
                .ok()?,
        ))
    }

    /// Get an f64 field.
    #[inline]
    fn get_f64(&self, name: &str) -> Option<f64> {
        let (_, meta) = self.find_field(name).ok()?;
        if meta.type_tag != TAG_F64 || meta.data_len != 8 {
            return None;
        }
        Some(f64::from_le_bytes(
            self.data_buf()[meta.data_offset..meta.data_offset + 8]
                .try_into()
                .ok()?,
        ))
    }

    /// Get a bool field.
    #[inline]
    fn get_bool(&self, name: &str) -> Option<bool> {
        let (_, meta) = self.find_field(name).ok()?;
        if meta.type_tag != TAG_BOOL || meta.data_len != 1 {
            return None;
        }
        Some(self.data_buf()[meta.data_offset] != 0)
    }

    /// Get raw field reference (zero-copy).
    #[inline]
    fn get_raw(&self, name: &str) -> Option<FieldRef<'_>> {
        let (_, meta) = self.find_field(name).ok()?;
        let data = &self.data_buf()[meta.data_offset..meta.data_offset + meta.data_len];
        Some(FieldRef {
            name_hash: meta.name_hash,
            type_tag: meta.type_tag,
            data,
        })
    }

    /// Get any field as a value (deserializes nested CBOR if needed).
    /// Specify the value type using turbofish syntax: `get_field::<SpookyValue>("name")`.
    #[inline]
    fn get_field<V: crate::deserialization::RecordDeserialize>(&self, name: &str) -> Option<V> {
        let field = self.get_raw(name)?;
        crate::deserialization::decode_field(field)
    }

    /// Get a numeric field as f64 (converting i64/u64 if needed).
    fn get_number_as_f64(&self, name: &str) -> Option<f64> {
        let (_, meta) = self.find_field(name).ok()?;
        match meta.type_tag {
            TAG_F64 | TAG_I64 | TAG_U64 if meta.data_len == 8 => {}
            _ => return None,
        }
        let bytes: [u8; 8] = self.data_buf()[meta.data_offset..meta.data_offset + 8]
            .try_into()
            .ok()?;
        match meta.type_tag {
            TAG_F64 => Some(f64::from_le_bytes(bytes)),
            TAG_I64 => Some(i64::from_le_bytes(bytes) as f64),
            TAG_U64 => Some(u64::from_le_bytes(bytes) as f64),
            _ => unreachable!(),
        }
    }

    /// Convert to SpookyValue (iterator-based full conversion placeholder).
    /// Note: Keys are not recoverable from hashes in the current format.
    fn to_value(&self) -> SpookyValue {
        SpookyValue::Null // Placeholder as per parity plan constraint
    }

    /// Check if a field exists.
    #[inline]
    fn has_field(&self, name: &str) -> bool {
        self.find_field(name).is_ok()
    }

    /// Get the type tag for a field.
    #[inline]
    fn field_type(&self, name: &str) -> Option<u8> {
        self.find_field(name).ok().map(|(_, m)| m.type_tag)
    }

    // ════════════════════════════════════════════════════════════════════════
    // FieldSlot — O(1) cached access
    // ════════════════════════════════════════════════════════════════════════
    /// Resolve a field by name into a cached FieldSlot.
    ///
    /// This performs one O(log n) lookup and caches all metadata needed for
    /// future O(1) access via `get_*_at` and `set_*_at` methods.
    ///
    /// The returned slot is valid until a layout-changing operation
    /// (add_field, remove_field, or variable-length splice). Staleness
    /// is checked via debug assertions in all `_at` methods.
    fn resolve(&self, name: &str) -> Option<FieldSlot> {
        let (index_pos, meta) = self.find_field(name).ok()?;
        Some(FieldSlot {
            index_pos,
            data_offset: meta.data_offset,
            data_len: meta.data_len,
            type_tag: meta.type_tag,
            generation: self.generation(),
        })
    }

    /// Get an i64 field using a cached FieldSlot. ~2-3ns vs ~10ns for by-name.
    #[inline]
    fn get_i64_at(&self, slot: &FieldSlot) -> Option<i64> {
        debug_assert_eq!(slot.generation, self.generation(), "stale FieldSlot");
        if slot.type_tag != TAG_I64 || slot.data_len != 8 {
            return None;
        }
        Some(i64::from_le_bytes(
            self.data_buf()[slot.data_offset..slot.data_offset + 8]
                .try_into()
                .ok()?,
        ))
    }

    /// Get a u64 field using a cached FieldSlot.
    #[inline]
    fn get_u64_at(&self, slot: &FieldSlot) -> Option<u64> {
        debug_assert_eq!(slot.generation, self.generation(), "stale FieldSlot");
        if slot.type_tag != TAG_U64 || slot.data_len != 8 {
            return None;
        }
        Some(u64::from_le_bytes(
            self.data_buf()[slot.data_offset..slot.data_offset + 8]
                .try_into()
                .ok()?,
        ))
    }

    /// Get an f64 field using a cached FieldSlot.
    #[inline]
    fn get_f64_at(&self, slot: &FieldSlot) -> Option<f64> {
        debug_assert_eq!(slot.generation, self.generation(), "stale FieldSlot");
        if slot.type_tag != TAG_F64 || slot.data_len != 8 {
            return None;
        }
        Some(f64::from_le_bytes(
            self.data_buf()[slot.data_offset..slot.data_offset + 8]
                .try_into()
                .ok()?,
        ))
    }

    /// Get a bool field using a cached FieldSlot.
    #[inline]
    fn get_bool_at(&self, slot: &FieldSlot) -> Option<bool> {
        debug_assert_eq!(slot.generation, self.generation(), "stale FieldSlot");
        if slot.type_tag != TAG_BOOL || slot.data_len != 1 {
            return None;
        }
        Some(self.data_buf()[slot.data_offset] != 0)
    }

    /// Get a string field using a cached FieldSlot (zero-copy).
    #[inline]
    fn get_str_at(&self, slot: &FieldSlot) -> Option<&str> {
        debug_assert_eq!(slot.generation, self.generation(), "stale FieldSlot");
        if slot.type_tag != TAG_STR {
            return None;
        }
        std::str::from_utf8(&self.data_buf()[slot.data_offset..slot.data_offset + slot.data_len])
            .ok()
    }
}
