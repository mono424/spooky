use super::error::RecordError;
use super::spooky_value::{SpookyNumber, SpookyValue};
use super::types::*;
use arrayvec::ArrayVec;
use smol_str::SmolStr;
use std::collections::BTreeMap;
use xxhash_rust::const_xxh64::xxh64;

// ─── RecordSerialize Trait ──────────────────────────────────────────────────

/// Trait for value types that can be serialized into the binary record format.
///
/// This trait abstracts over different value representations (SpookyValue,
/// serde_json::Value, cbor4ii::core::Value) allowing them to be stored in the
/// same hybrid binary format.
pub trait RecordSerialize: serde::Serialize {
    /// Check if this value is null.
    fn is_null(&self) -> bool;

    /// Extract a boolean value, if this is a boolean.
    fn as_bool(&self) -> Option<bool>;

    /// Extract an i64 value, if this is an i64.
    fn as_i64(&self) -> Option<i64>;

    /// Extract a u64 value, if this is a u64.
    fn as_u64(&self) -> Option<u64>;

    /// Extract an f64 value, if this is an f64.
    fn as_f64(&self) -> Option<f64>;

    /// Extract a string slice, if this is a string.
    fn as_str(&self) -> Option<&str>;

    /// Check if this value is nested (array or object).
    fn is_nested(&self) -> bool;
}

// ─── RecordSerialize for SpookyValue ────────────────────────────────────────

impl RecordSerialize for SpookyValue {
    #[inline]
    fn is_null(&self) -> bool {
        matches!(self, SpookyValue::Null)
    }

    #[inline]
    fn as_bool(&self) -> Option<bool> {
        match self {
            SpookyValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    #[inline]
    fn as_i64(&self) -> Option<i64> {
        match self {
            SpookyValue::Number(SpookyNumber::I64(i)) => Some(*i),
            _ => None,
        }
    }

    #[inline]
    fn as_u64(&self) -> Option<u64> {
        match self {
            SpookyValue::Number(SpookyNumber::U64(u)) => Some(*u),
            _ => None,
        }
    }

    #[inline]
    fn as_f64(&self) -> Option<f64> {
        match self {
            SpookyValue::Number(SpookyNumber::F64(f)) => Some(*f),
            _ => None,
        }
    }

    #[inline]
    fn as_str(&self) -> Option<&str> {
        match self {
            SpookyValue::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    #[inline]
    fn is_nested(&self) -> bool {
        matches!(self, SpookyValue::Array(_) | SpookyValue::Object(_))
    }
}

// ─── RecordSerialize for serde_json::Value ──────────────────────────────────

impl RecordSerialize for serde_json::Value {
    #[inline]
    fn is_null(&self) -> bool {
        matches!(self, serde_json::Value::Null)
    }

    #[inline]
    fn as_bool(&self) -> Option<bool> {
        self.as_bool()
    }

    #[inline]
    fn as_i64(&self) -> Option<i64> {
        self.as_i64()
    }

    #[inline]
    fn as_u64(&self) -> Option<u64> {
        self.as_u64()
    }

    #[inline]
    fn as_f64(&self) -> Option<f64> {
        self.as_f64()
    }

    #[inline]
    fn as_str(&self) -> Option<&str> {
        self.as_str()
    }

    #[inline]
    fn is_nested(&self) -> bool {
        matches!(
            self,
            serde_json::Value::Array(_) | serde_json::Value::Object(_)
        )
    }
}

// ─── RecordSerialize for cbor4ii::core::Value ───────────────────────────────

impl RecordSerialize for cbor4ii::core::Value {
    #[inline]
    fn is_null(&self) -> bool {
        matches!(self, cbor4ii::core::Value::Null)
    }

    #[inline]
    fn as_bool(&self) -> Option<bool> {
        match self {
            cbor4ii::core::Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    #[inline]
    fn as_i64(&self) -> Option<i64> {
        match self {
            cbor4ii::core::Value::Integer(i) => i64::try_from(*i).ok(),
            _ => None,
        }
    }

    #[inline]
    fn as_u64(&self) -> Option<u64> {
        match self {
            cbor4ii::core::Value::Integer(i) => u64::try_from(*i).ok(),
            _ => None,
        }
    }

    #[inline]
    fn as_f64(&self) -> Option<f64> {
        match self {
            cbor4ii::core::Value::Float(f) => Some(*f),
            cbor4ii::core::Value::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }

    #[inline]
    fn as_str(&self) -> Option<&str> {
        match self {
            cbor4ii::core::Value::Text(s) => Some(s.as_str()),
            _ => None,
        }
    }

    #[inline]
    fn is_nested(&self) -> bool {
        matches!(
            self,
            cbor4ii::core::Value::Array(_) | cbor4ii::core::Value::Map(_)
        )
    }
}

// ─── RecordSerialize for &T ─────────────────────────────────────────────────

/// Blanket implementation for references — allows passing &SpookyValue, etc.
impl<T: RecordSerialize> RecordSerialize for &T {
    #[inline]
    fn is_null(&self) -> bool {
        (**self).is_null()
    }

    #[inline]
    fn as_bool(&self) -> Option<bool> {
        (**self).as_bool()
    }

    #[inline]
    fn as_i64(&self) -> Option<i64> {
        (**self).as_i64()
    }

    #[inline]
    fn as_u64(&self) -> Option<u64> {
        (**self).as_u64()
    }

    #[inline]
    fn as_f64(&self) -> Option<f64> {
        (**self).as_f64()
    }

    #[inline]
    fn as_str(&self) -> Option<&str> {
        (**self).as_str()
    }

    #[inline]
    fn is_nested(&self) -> bool {
        (**self).is_nested()
    }
}

// ─── Writer ─────────────────────────────────────────────────────────────────

/// Serialize a SpookyValue::Object into the hybrid binary format.
/// Flat fields are stored as native bytes, nested objects/arrays as CBOR.
///
/// **IMPORTANT**: The index is sorted by name_hash. This is required for
/// O(log n) binary search in both SpookyRecord and SpookyRecordMut.

/// Serialize a single field value into (bytes, type_tag).
#[inline]
pub fn write_field_into<V: RecordSerialize>(
    buf: &mut Vec<u8>,
    value: &V,
) -> Result<u8, RecordError> {
    Ok(if value.is_null() {
        TAG_NULL
    } else if let Some(b) = value.as_bool() {
        buf.push(b as u8);
        TAG_BOOL
    } else if let Some(i) = value.as_i64() {
        // i64 — reserve once, write directly
        buf.reserve(8);
        let len = buf.len();
        let bytes = i.to_le_bytes();
        // SAFETY: we just reserved 8 bytes
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf.as_mut_ptr().add(len), 8);
            buf.set_len(len + 8);
        }
        TAG_I64
    } else if let Some(u) = value.as_u64() {
        // u64 — reserve once, write directly
        buf.reserve(8);
        let len = buf.len();
        let bytes = u.to_le_bytes();
        // SAFETY: we just reserved 8 bytes
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf.as_mut_ptr().add(len), 8);
            buf.set_len(len + 8);
        }
        TAG_U64
    } else if let Some(f) = value.as_f64() {
        // f64 — reserve once, write directly
        buf.reserve(8);
        let len = buf.len();
        let bytes = f.to_le_bytes();
        // SAFETY: we just reserved 8 bytes
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf.as_mut_ptr().add(len), 8);
            buf.set_len(len + 8);
        }
        TAG_F64
    } else if let Some(s) = value.as_str() {
        buf.extend_from_slice(s.as_bytes());
        TAG_STR
    } else if value.is_nested() {
        // Array or Object — serialize as CBOR using serde::Serialize
        cbor4ii::serde::to_writer(&mut *buf, value)
            .map_err(|e| RecordError::CborError(e.to_string()))?;
        TAG_NESTED_CBOR
    } else {
        // Unknown type — treat as null
        TAG_NULL
    })
}

pub fn prepare_buf<V: RecordSerialize>(
    map: &BTreeMap<SmolStr, V>,
    buf: &mut Vec<u8>,
    field_count: usize,
) -> Result<(), RecordError> {
    // 3. Sort
    // Collect references & hashes to avoid unnecessary data copies.
    // // Stack-allocated sort buffer — no heap allocation for ≤32 fields
    // //TODO: has to be check if this could be panic in normal sitations
    let mut entries: ArrayVec<(&V, u64), 32> = ArrayVec::new();

    for (key, value) in map.iter() {
        // Compute the hash for the key
        let hash = xxh64(key.as_bytes(), 0);
        entries
            .try_push((value, hash))
            .map_err(|_| RecordError::TooManyFields)?;
    }

    // Sort for O(log n) lookup in the reader
    entries.sort_unstable_by_key(|(_, hash)| *hash);

    // Write header (field count)
    buf[0..4].copy_from_slice(&(field_count as u32).to_le_bytes());

    // 4. Loop & Write
    for (i, (value, hash)) in entries.iter().enumerate() {
        // A. Append data to value area
        let data_offset = buf.len();
        let tag = write_field_into(buf, value)?;
        let data_length = buf.len() - data_offset;

        // B. Fill in the index entry
        // All arithmetic must use usize
        let idx = HEADER_SIZE + i * INDEX_ENTRY_SIZE;
        let entry = &mut buf[idx..idx + INDEX_ENTRY_SIZE];
        entry[0..8].copy_from_slice(&hash.to_le_bytes());
        entry[8..12].copy_from_slice(&(data_offset as u32).to_le_bytes());
        entry[12..16].copy_from_slice(&(data_length as u32).to_le_bytes());
        entry[16] = tag;
    }
    Ok(())
}

// ════════════════════════════════════════════════════════════════════════
// Serializations patterns
// ════════════════════════════════════════════════════════════════════════

pub fn serialize<V: RecordSerialize>(
    map: &BTreeMap<SmolStr, V>,
) -> Result<(Vec<u8>, usize), RecordError> {
    let field_count = map.len();

    // NOTE: All arithmetic must use usize
    let index_size = field_count * INDEX_ENTRY_SIZE;
    let data_start = HEADER_SIZE + index_size;

    // 2. Prepare buffer (rough capacity estimate)
    let mut buf: Vec<u8> = Vec::with_capacity(data_start + field_count * 32);
    // CRITICAL: resize fills the header/index area with 0,
    // so we can write by index (buf[idx] = ...) immediately.
    buf.resize(data_start, 0);

    prepare_buf(&map, &mut buf, field_count)?;

    // 5. Return
    Ok((buf, field_count))
}

/// Serialize a SpookyValue::Object into the hybrid binary format.
/// Flat fields are stored as native bytes, nested objects/arrays as CBOR.
///
/// **IMPORTANT**: The index is sorted by name_hash. This is required for
/// O(log n) binary search in both SpookyRecord and SpookyRecordMut.
pub fn from_spooky(data: &SpookyValue) -> Result<(Vec<u8>, usize), RecordError> {
    let map = match data {
        SpookyValue::Object(map) => map,
        _ => return Err(RecordError::InvalidBuffer),
    };

    let (buf, field_count) = serialize::<SpookyValue>(map)?;
    Ok((buf, field_count))
}

/// Serialize a cbor4ii::core::Value::Map into the hybrid binary format.
pub fn from_cbor(data: &cbor4ii::core::Value) -> Result<(Vec<u8>, usize), RecordError> {
    let entries = match data {
        cbor4ii::core::Value::Map(entries) => entries,
        _ => return Err(RecordError::InvalidBuffer),
    };

    let mut map = BTreeMap::new();
    for (k, v) in entries {
        let key_str = match k {
            cbor4ii::core::Value::Text(s) => SmolStr::from(s),
            _ => return Err(RecordError::CborError("Key must be a string".into())),
        };
        map.insert(key_str, v.clone());
    }

    serialize(&map)
}

/// Create a mutable record by taking ownership of an existing serialized buffer.
///
/// The buffer **must** have a sorted index (produced by `serialize_record()`,
/// `from_spooky_value()`, or a previous `into_bytes()`).
/// Validate a byte slice and extract field_count.
pub fn from_bytes(buf: &[u8]) -> Result<(&[u8], usize), RecordError> {
    if buf.len() < HEADER_SIZE {
        return Err(RecordError::InvalidBuffer);
    }
    let field_count = u32::from_le_bytes(
        buf[0..4]
            .try_into()
            .map_err(|_| RecordError::InvalidBuffer)?,
    ) as usize;
    let min_size = HEADER_SIZE + field_count * INDEX_ENTRY_SIZE;
    if buf.len() < min_size {
        return Err(RecordError::InvalidBuffer);
    }
    Ok((buf, field_count))
}

/// Serialize a SpookyValue::Object into a reusable buffer.
///
/// Identical to `serialize`, but reuses the caller's Vec to eliminate
/// allocations when serializing many records in sequence (sync ingestion,
/// snapshot rebuild). The buffer is cleared but retains its capacity.
///
/// **IMPORTANT**: The index is sorted by name_hash.
pub fn serialize_into<V: RecordSerialize>(
    map: &BTreeMap<SmolStr, V>,
    buf: &mut Vec<u8>,
) -> Result<usize, RecordError> {
    let field_count = map.len();
    // NOTE: All arithmetic must use usize
    let index_size = field_count * INDEX_ENTRY_SIZE;
    let data_start = HEADER_SIZE + index_size;

    // Reuse buffer — clears but retains capacity
    buf.clear();
    buf.reserve(data_start + field_count * 16);
    buf.resize(data_start, 0);

    prepare_buf(&map, buf, field_count)?;
    // 5. Return
    Ok(field_count)
}

pub fn serialize_into_buf(data: &SpookyValue, buf: &mut Vec<u8>) -> Result<(), RecordError> {
    let map = match data {
        SpookyValue::Object(map) => map,
        _ => return Err(RecordError::InvalidBuffer),
    };

    let _ = serialize_into::<SpookyValue>(map, buf)?;

    Ok(())
}
