use super::read_op::SpookyReadable;
use super::record_mut::SpookyRecordMut;
use crate::error::RecordError;
use crate::serialization::write_field_into;
use crate::spooky_value::SpookyValue;
use crate::types::*;

impl SpookyRecordMut {
    // ════════════════════════════════════════════════════════════════════════
    // Internal: index writes
    // ════════════════════════════════════════════════════════════════════════

    #[inline]
    fn write_index_offset(&mut self, i: usize, offset: usize) {
        let idx = HEADER_SIZE + i * INDEX_ENTRY_SIZE;
        self.data_buf[idx + 8..idx + 12].copy_from_slice(&(offset as u32).to_le_bytes());
    }

    #[inline]
    fn write_index_length(&mut self, i: usize, length: usize) {
        let idx = HEADER_SIZE + i * INDEX_ENTRY_SIZE;
        self.data_buf[idx + 12..idx + 16].copy_from_slice(&(length as u32).to_le_bytes());
    }

    #[inline]
    fn write_index_tag(&mut self, i: usize, tag: u8) {
        let idx = HEADER_SIZE + i * INDEX_ENTRY_SIZE;
        self.data_buf[idx + 16] = tag;
    }

    #[inline]
    fn read_index_offset(&self, i: usize) -> usize {
        let idx = HEADER_SIZE + i * INDEX_ENTRY_SIZE;
        u32::from_le_bytes(self.data_buf[idx + 8..idx + 12].try_into().unwrap()) as usize
    }

    /// After a splice at `splice_offset`, shift data_offsets for all fields
    /// whose data starts STRICTLY AFTER `splice_offset` by `delta` bytes.
    /// The field at `skip_pos` (the one we just modified) is excluded.
    fn fixup_offsets_after_splice(&mut self, skip_pos: usize, splice_offset: usize, delta: isize) {
        for i in 0..self.field_count as usize {
            if i == skip_pos {
                continue;
            }
            let offset = self.read_index_offset(i);
            if offset > splice_offset {
                let new_offset = (offset as isize + delta) as usize;
                self.write_index_offset(i, new_offset);
            }
        }
    }

    // ════════════════════════════════════════════════════════════════════════
    // Internal: buffer splice
    // ════════════════════════════════════════════════════════════════════════

    /// Replace `old_len` bytes at `offset` with `new_data`.
    /// Handles grow, shrink, and same-size cases.
    fn splice_data(&mut self, offset: usize, old_len: usize, new_data: &[u8]) {
        let new_len = new_data.len();
        let old_end = offset + old_len;
        let tail_len = self.data_buf.len() - old_end;

        if new_len == old_len {
            self.data_buf[offset..offset + new_len].copy_from_slice(new_data);
        } else if new_len > old_len {
            let growth = new_len - old_len;
            self.data_buf.resize(self.data_buf.len() + growth, 0);
            // Shift tail right
            self.data_buf
                .copy_within(old_end..old_end + tail_len, old_end + growth);
            self.data_buf[offset..offset + new_len].copy_from_slice(new_data);
        } else {
            let shrink = old_len - new_len;
            self.data_buf[offset..offset + new_len].copy_from_slice(new_data);
            // Shift tail left
            self.data_buf
                .copy_within(old_end..old_end + tail_len, old_end - shrink);
            self.data_buf.truncate(self.data_buf.len() - shrink);
        }
    }

    // ════════════════════════════════════════════════════════════════════════
    // Typed setters — FAST PATH (in-place, zero allocation)
    // ════════════════════════════════════════════════════════════════════════

    /// Set an i64 field. In-place overwrite, ~20ns. Zero allocation.
    #[inline]
    pub fn set_i64(&mut self, name: &str, value: i64) -> Result<(), RecordError> {
        let (_, meta) = self.find_field(name)?;
        if meta.type_tag != TAG_I64 {
            return Err(RecordError::TypeMismatch {
                expected: TAG_I64,
                actual: meta.type_tag,
            });
        }
        self.data_buf[meta.data_offset..meta.data_offset + 8].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Set a u64 field. In-place overwrite, ~20ns. Zero allocation.
    #[inline]
    pub fn set_u64(&mut self, name: &str, value: u64) -> Result<(), RecordError> {
        let (_, meta) = self.find_field(name)?;
        if meta.type_tag != TAG_U64 {
            return Err(RecordError::TypeMismatch {
                expected: TAG_U64,
                actual: meta.type_tag,
            });
        }
        self.data_buf[meta.data_offset..meta.data_offset + 8].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Set an f64 field. In-place overwrite, ~20ns. Zero allocation.
    #[inline]
    pub fn set_f64(&mut self, name: &str, value: f64) -> Result<(), RecordError> {
        let (_, meta) = self.find_field(name)?;
        if meta.type_tag != TAG_F64 {
            return Err(RecordError::TypeMismatch {
                expected: TAG_F64,
                actual: meta.type_tag,
            });
        }
        self.data_buf[meta.data_offset..meta.data_offset + 8].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Set a bool field. In-place overwrite, ~18ns. Zero allocation.
    #[inline]
    pub fn set_bool(&mut self, name: &str, value: bool) -> Result<(), RecordError> {
        let (_, meta) = self.find_field(name)?;
        if meta.type_tag != TAG_BOOL {
            return Err(RecordError::TypeMismatch {
                expected: TAG_BOOL,
                actual: meta.type_tag,
            });
        }
        self.data_buf[meta.data_offset] = value as u8;
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════════
    // String setters
    // ════════════════════════════════════════════════════════════════════════

    /// Set a string field. In-place if same byte length, splice if different.
    ///
    /// ~22ns for same length, ~150-350ns for different length.
    pub fn set_str(&mut self, name: &str, value: &str) -> Result<(), RecordError> {
        let (pos, meta) = self.find_field(name)?;
        if meta.type_tag != TAG_STR {
            return Err(RecordError::TypeMismatch {
                expected: TAG_STR,
                actual: meta.type_tag,
            });
        }

        let new_bytes = value.as_bytes();

        if new_bytes.len() == meta.data_len {
            // Fast path: same length, direct overwrite
            self.data_buf[meta.data_offset..meta.data_offset + meta.data_len]
                .copy_from_slice(new_bytes);
        } else {
            // Splice path
            let delta = new_bytes.len() as isize - meta.data_len as isize;
            self.splice_data(meta.data_offset, meta.data_len, new_bytes);
            self.write_index_length(pos, new_bytes.len());
            self.fixup_offsets_after_splice(pos, meta.data_offset, delta);
            self.generation += 1; // Layout changed
        }
        Ok(())
    }

    /// Set a string field only if the new value has the exact same byte length.
    /// Returns `RecordError::LengthMismatch` otherwise. Guaranteed zero-allocation.
    #[inline]
    pub fn set_str_exact(&mut self, name: &str, value: &str) -> Result<(), RecordError> {
        let (_, meta) = self.find_field(name)?;
        if meta.type_tag != TAG_STR {
            return Err(RecordError::TypeMismatch {
                expected: TAG_STR,
                actual: meta.type_tag,
            });
        }
        let new_bytes = value.as_bytes();
        if new_bytes.len() != meta.data_len {
            return Err(RecordError::LengthMismatch {
                expected: meta.data_len,
                actual: new_bytes.len(),
            });
        }
        self.data_buf[meta.data_offset..meta.data_offset + meta.data_len]
            .copy_from_slice(new_bytes);
        Ok(())
    }

    // ════════════════════════════════════════════════════════════════════════
    // Generic setter — handles any type/size change
    // ════════════════════════════════════════════════════════════════════════

    /// Set any field to any value. Automatically picks the optimal path:
    /// - Same size → in-place overwrite (~25ns)
    /// - Different size → splice + offset fixup (~200-500ns)
    pub fn set_field<V: crate::serialization::RecordSerialize>(&mut self, name: &str, value: &V) -> Result<(), RecordError> {
        let (pos, meta) = self.find_field(name)?;
        let mut new_bytes = Vec::new();
        let new_tag = write_field_into(&mut new_bytes, value)?;

        if new_bytes.len() == meta.data_len {
            // Fast path: same size
            if !new_bytes.is_empty() {
                self.data_buf[meta.data_offset..meta.data_offset + meta.data_len]
                    .copy_from_slice(&new_bytes);
            }
            if new_tag != meta.type_tag {
                self.write_index_tag(pos, new_tag);
            }
        } else {
            // Splice path
            let delta = new_bytes.len() as isize - meta.data_len as isize;
            self.splice_data(meta.data_offset, meta.data_len, &new_bytes);
            self.write_index_length(pos, new_bytes.len());
            self.write_index_tag(pos, new_tag);
            self.fixup_offsets_after_splice(pos, meta.data_offset, delta);
            self.generation += 1; // Layout changed
        }
        Ok(())
    }

    /// Set a field to Null.
    pub fn set_null(&mut self, name: &str) -> Result<(), RecordError> {
        self.set_field(name, &SpookyValue::Null)
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
    /// Set an i64 field using a cached FieldSlot. In-place, ~20ns.
    #[inline]
    pub fn set_i64_at(&mut self, slot: &FieldSlot, value: i64) -> Result<(), RecordError> {
        if slot.type_tag != TAG_I64 {
            return Err(RecordError::TypeMismatch {
                expected: TAG_I64,
                actual: slot.type_tag,
            });
        }
        self.data_buf[slot.data_offset..slot.data_offset + 8].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Set a u64 field using a cached FieldSlot. In-place, ~20ns.
    #[inline]
    pub fn set_u64_at(&mut self, slot: &FieldSlot, value: u64) -> Result<(), RecordError> {
        if slot.type_tag != TAG_U64 {
            return Err(RecordError::TypeMismatch {
                expected: TAG_U64,
                actual: slot.type_tag,
            });
        }
        self.data_buf[slot.data_offset..slot.data_offset + 8].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Set an f64 field using a cached FieldSlot. In-place, ~20ns.
    #[inline]
    pub fn set_f64_at(&mut self, slot: &FieldSlot, value: f64) -> Result<(), RecordError> {
        debug_assert_eq!(slot.generation, self.generation, "stale FieldSlot");
        if slot.type_tag != TAG_F64 {
            return Err(RecordError::TypeMismatch {
                expected: TAG_F64,
                actual: slot.type_tag,
            });
        }
        self.data_buf[slot.data_offset..slot.data_offset + 8].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Set a bool field using a cached FieldSlot. In-place, ~18ns.
    #[inline]
    pub fn set_bool_at(&mut self, slot: &FieldSlot, value: bool) -> Result<(), RecordError> {
        debug_assert_eq!(slot.generation, self.generation, "stale FieldSlot");
        if slot.type_tag != TAG_BOOL {
            return Err(RecordError::TypeMismatch {
                expected: TAG_BOOL,
                actual: slot.type_tag,
            });
        }
        self.data_buf[slot.data_offset] = value as u8;
        Ok(())
    }

    /// Set a string field using a cached FieldSlot.
    ///
    /// **Conservative strategy**: Only accepts same-byte-length writes.
    /// Returns `LengthMismatch` if the new value has a different byte length.
    /// Caller should fall back to `set_str` + re-resolve on mismatch.
    ///
    /// Same-length writes are in-place (~22ns) and don't invalidate the slot.
    #[inline]
    pub fn set_str_at(&mut self, slot: &FieldSlot, value: &str) -> Result<(), RecordError> {
        debug_assert_eq!(slot.generation, self.generation, "stale FieldSlot");
        if slot.type_tag != TAG_STR {
            return Err(RecordError::TypeMismatch {
                expected: TAG_STR,
                actual: slot.type_tag,
            });
        }
        let new_bytes = value.as_bytes();
        if new_bytes.len() != slot.data_len {
            return Err(RecordError::LengthMismatch {
                expected: slot.data_len,
                actual: new_bytes.len(),
            });
        }
        self.data_buf[slot.data_offset..slot.data_offset + slot.data_len]
            .copy_from_slice(new_bytes);
        Ok(())
    }
}
