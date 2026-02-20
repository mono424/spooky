use super::SpookyRecord;
use super::read_op::SpookyReadable;
use crate::types::*;

pub struct SpookyRecordMut {
    pub data_buf: Vec<u8>,
    pub field_count: usize,
    /// Generation counter, bumped on every layout-changing mutation.
    /// Used to detect stale FieldSlots.
    pub generation: usize,
}

impl SpookyRecordMut {
    pub fn new(data_buf: Vec<u8>, field_count: usize) -> Self {
        Self {
            data_buf,
            field_count,
            generation: 0,
        }
    }

    /// Create a new empty mutable record.
    pub fn new_empty() -> Self {
        let mut data_buf = vec![0u8; HEADER_SIZE];
        data_buf[0..4].copy_from_slice(&0u32.to_le_bytes());
        Self {
            data_buf,
            field_count: 0,
            generation: 0,
        }
    }

    #[inline]
    pub fn as_record(&self) -> SpookyRecord<'_> {
        SpookyRecord::new(&self.data_buf, self.field_count)
    }

    /// Find the sorted insertion position for a new hash.
    pub fn find_insert_pos(&self, hash: u64) -> usize {
        let n = self.field_count as usize;
        let mut lo = 0usize;
        let mut hi = n;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.read_hash(mid) < hash {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }
}

impl SpookyReadable for SpookyRecordMut {
    #[inline]
    fn data_buf(&self) -> &[u8] {
        &self.data_buf
    }
    #[inline]
    fn field_count(&self) -> usize {
        self.field_count
    }

    #[inline]
    fn iter_fields(&self) -> FieldIter<'_> {
        let view = SpookyRecord {
            data_buf: &self.data_buf,
            field_count: self.field_count,
        };

        FieldIter {
            record: view,
            pos: 0,
        }
    }

    #[inline]
    fn generation(&self) -> usize {
        self.generation
    }
}



/* TODO: There are currently missing methods:
    // ════════════════════════════════════════════════════════════════════════
    // Finalize
    // ════════════════════════════════════════════════════════════════════════

    /// Consume and return the underlying buffer. Use this to write to redb.
    #[inline]
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    /// Borrow the underlying buffer.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    /// Total byte size of the record.
    #[inline]
    pub fn byte_len(&self) -> usize {
        self.buf.len()
    }
}
*/