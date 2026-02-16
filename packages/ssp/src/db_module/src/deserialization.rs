use super::spooky_value::{SpookyNumber, SpookyValue};
use super::types::*;
use smol_str::SmolStr;

// ─── RecordDeserialize Trait ────────────────────────────────────────────────

/// Trait for value types that can be deserialized from the binary record format.
///
/// This trait abstracts over different value representations (SpookyValue,
/// serde_json::Value, cbor4ii::core::Value) allowing them to be reconstructed
/// from the same hybrid binary format.
pub trait RecordDeserialize: Sized {
    /// Construct a null value.
    fn from_null() -> Self;

    /// Construct a boolean value.
    fn from_bool(b: bool) -> Self;

    /// Construct an i64 value.
    fn from_i64(v: i64) -> Self;

    /// Construct a u64 value.
    fn from_u64(v: u64) -> Self;

    /// Construct an f64 value.
    fn from_f64(v: f64) -> Self;

    /// Construct a string value.
    fn from_str(s: &str) -> Self;

    /// Deserialize from CBOR bytes (for nested objects/arrays).
    fn from_cbor_bytes(data: &[u8]) -> Option<Self>;
}

// ─── RecordDeserialize for SpookyValue ──────────────────────────────────────

impl RecordDeserialize for SpookyValue {
    #[inline]
    fn from_null() -> Self {
        SpookyValue::Null
    }

    #[inline]
    fn from_bool(b: bool) -> Self {
        SpookyValue::Bool(b)
    }

    #[inline]
    fn from_i64(v: i64) -> Self {
        SpookyValue::Number(SpookyNumber::I64(v))
    }

    #[inline]
    fn from_u64(v: u64) -> Self {
        SpookyValue::Number(SpookyNumber::U64(v))
    }

    #[inline]
    fn from_f64(v: f64) -> Self {
        SpookyValue::Number(SpookyNumber::F64(v))
    }

    #[inline]
    fn from_str(s: &str) -> Self {
        SpookyValue::Str(SmolStr::from(s))
    }

    #[inline]
    fn from_cbor_bytes(data: &[u8]) -> Option<Self> {
        let cbor_val: cbor4ii::core::Value = cbor4ii::serde::from_slice(data).ok()?;
        Some(SpookyValue::from(cbor_val))
    }
}

// ─── RecordDeserialize for serde_json::Value ────────────────────────────────

impl RecordDeserialize for serde_json::Value {
    #[inline]
    fn from_null() -> Self {
        serde_json::Value::Null
    }

    #[inline]
    fn from_bool(b: bool) -> Self {
        serde_json::Value::Bool(b)
    }

    #[inline]
    fn from_i64(v: i64) -> Self {
        serde_json::Value::Number(v.into())
    }

    #[inline]
    fn from_u64(v: u64) -> Self {
        serde_json::Value::Number(v.into())
    }

    #[inline]
    fn from_f64(v: f64) -> Self {
        serde_json::Number::from_f64(v)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null)
    }

    #[inline]
    fn from_str(s: &str) -> Self {
        serde_json::Value::String(s.to_string())
    }

    #[inline]
    fn from_cbor_bytes(data: &[u8]) -> Option<Self> {
        cbor4ii::serde::from_slice(data).ok()
    }
}

// ─── RecordDeserialize for cbor4ii::core::Value ─────────────────────────────

impl RecordDeserialize for cbor4ii::core::Value {
    #[inline]
    fn from_null() -> Self {
        cbor4ii::core::Value::Null
    }

    #[inline]
    fn from_bool(b: bool) -> Self {
        cbor4ii::core::Value::Bool(b)
    }

    #[inline]
    fn from_i64(v: i64) -> Self {
        cbor4ii::core::Value::Integer(v as i128)
    }

    #[inline]
    fn from_u64(v: u64) -> Self {
        cbor4ii::core::Value::Integer(v as i128)
    }

    #[inline]
    fn from_f64(v: f64) -> Self {
        cbor4ii::core::Value::Float(v)
    }

    #[inline]
    fn from_str(s: &str) -> Self {
        cbor4ii::core::Value::Text(s.to_string())
    }

    #[inline]
    fn from_cbor_bytes(data: &[u8]) -> Option<Self> {
        cbor4ii::serde::from_slice(data).ok()
    }
}

// ─── Decode Field ───────────────────────────────────────────────────────────

/// Decode a raw field reference into any value type that implements RecordDeserialize.
#[inline]
pub fn decode_field<V: RecordDeserialize>(field: FieldRef) -> Option<V> {
    Some(match field.type_tag {
        TAG_NULL => V::from_null(),
        TAG_BOOL => V::from_bool(*field.data.first()? != 0),
        TAG_I64 => {
            let bytes: [u8; 8] = field.data.try_into().ok()?;
            V::from_i64(i64::from_le_bytes(bytes))
        }
        TAG_F64 => {
            let bytes: [u8; 8] = field.data.try_into().ok()?;
            V::from_f64(f64::from_le_bytes(bytes))
        }
        TAG_U64 => {
            let bytes: [u8; 8] = field.data.try_into().ok()?;
            V::from_u64(u64::from_le_bytes(bytes))
        }
        TAG_STR => V::from_str(std::str::from_utf8(field.data).ok()?),
        TAG_NESTED_CBOR => V::from_cbor_bytes(field.data)?,
        _ => return None,
    })
}
