//! Backing store for encoded row bytes.
//!
//! Rows are appended and never moved, so a stored [`RowSlot`] stays valid until
//! the slot is rewritten or compaction runs. Updates append a fresh copy and
//! repoint the slot; deletes drop the slot. Both are O(1), and the bytes they
//! orphan are tracked as `dead_bytes` for a later compaction pass.
//!
//! The trait exists so the *backing* can differ per platform while the encoding
//! stays shared. [`HeapArena`] is always compiled, which is what lets wasm32 —
//! the browser build, the Cloudflare Durable Object with its hard 128 MB
//! ceiling, and the Dart FFI dylib — get the encoding win even though none of
//! them can mmap a file. A file-backed implementation slots in behind the same
//! trait without any operator noticing.
//!
//! [`RowSlot`]: crate::circuit::row_table::RowSlot

/// Where a record lives: which segment, and the byte range within it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub seg: u16,
    pub off: u32,
    pub len: u32,
}

/// Append-only byte storage addressed by [`Span`].
pub trait Arena: std::fmt::Debug + Send + Sync {
    /// Append `bytes`, returning where they landed.
    fn append(&mut self, bytes: &[u8]) -> Span;

    /// Read back a span. Returns an empty slice if the span is out of range,
    /// so a corrupt index degrades to a missing row rather than a panic.
    fn get(&self, span: Span) -> &[u8];

    /// Note that a span is no longer referenced. Accounting only — the bytes
    /// stay until compaction.
    fn free(&mut self, span: Span);

    /// Bytes currently referenced by a live slot.
    fn live_bytes(&self) -> u64;

    /// Bytes orphaned by updates and deletes.
    fn dead_bytes(&self) -> u64;

    /// Total allocated capacity.
    fn capacity_bytes(&self) -> u64;

    /// Drop everything.
    fn clear(&mut self);
}

/// In-memory arena: one growable byte buffer.
///
/// A single segment is enough here because nothing needs to unmap or reclaim
/// address space; segmentation only earns its keep against a file-backed
/// implementation, which can then drop a whole segment at once.
#[derive(Debug, Default)]
pub struct HeapArena {
    buf: Vec<u8>,
    live: u64,
    dead: u64,
}

impl HeapArena {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(bytes: usize) -> Self {
        Self {
            buf: Vec::with_capacity(bytes),
            live: 0,
            dead: 0,
        }
    }
}

impl Arena for HeapArena {
    fn append(&mut self, bytes: &[u8]) -> Span {
        let off = self.buf.len() as u32;
        self.buf.extend_from_slice(bytes);
        self.live += bytes.len() as u64;
        Span {
            seg: 0,
            off,
            len: bytes.len() as u32,
        }
    }

    fn get(&self, span: Span) -> &[u8] {
        let start = span.off as usize;
        let end = start.saturating_add(span.len as usize);
        self.buf.get(start..end).unwrap_or(&[])
    }

    fn free(&mut self, span: Span) {
        let len = span.len as u64;
        self.live = self.live.saturating_sub(len);
        self.dead += len;
    }

    fn live_bytes(&self) -> u64 {
        self.live
    }

    fn dead_bytes(&self) -> u64 {
        self.dead
    }

    fn capacity_bytes(&self) -> u64 {
        self.buf.capacity() as u64
    }

    fn clear(&mut self) {
        self.buf.clear();
        self.live = 0;
        self.dead = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_read_back() {
        let mut a = HeapArena::new();
        let one = a.append(b"hello");
        let two = a.append(b"world!");
        assert_eq!(a.get(one), b"hello");
        assert_eq!(a.get(two), b"world!");
        assert_eq!(a.live_bytes(), 11);
        assert_eq!(a.dead_bytes(), 0);
    }

    #[test]
    fn appending_does_not_move_earlier_spans() {
        // Slots must stay valid across later appends; growth reallocating the
        // buffer is fine because spans are offsets, not pointers.
        let mut a = HeapArena::new();
        let first = a.append(b"first");
        for i in 0..1000u32 {
            a.append(&i.to_le_bytes());
        }
        assert_eq!(a.get(first), b"first");
    }

    #[test]
    fn free_moves_bytes_from_live_to_dead() {
        let mut a = HeapArena::new();
        let s = a.append(b"1234");
        a.append(b"56");
        a.free(s);
        assert_eq!(a.live_bytes(), 2);
        assert_eq!(a.dead_bytes(), 4);
        // Freed bytes are still readable — nothing has moved yet.
        assert_eq!(a.get(s), b"1234");
    }

    #[test]
    fn out_of_range_span_reads_empty_rather_than_panicking() {
        let a = HeapArena::new();
        let bogus = Span {
            seg: 0,
            off: 9_999,
            len: 10,
        };
        assert!(a.get(bogus).is_empty());

        let mut a = HeapArena::new();
        a.append(b"short");
        // A span that starts inside but runs past the end.
        assert!(a
            .get(Span {
                seg: 0,
                off: 3,
                len: 100
            })
            .is_empty());
        // A length that would overflow the addition.
        assert!(a
            .get(Span {
                seg: 0,
                off: u32::MAX,
                len: u32::MAX
            })
            .is_empty());
    }

    #[test]
    fn clear_resets_accounting() {
        let mut a = HeapArena::new();
        let s = a.append(b"data");
        a.free(s);
        a.clear();
        assert_eq!(a.live_bytes(), 0);
        assert_eq!(a.dead_bytes(), 0);
        assert!(a.get(s).is_empty());
    }
}
