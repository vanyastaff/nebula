//! The durable JSON-v1 encoding, independent of the canonical value-tree version.

use serde_json::Value;

use crate::error::ValidationError;

const JSON_V1_DOMAIN: &[u8] = b"nbschema-value-v";
const JSON_V1_VERSION: u16 = 1;
// This limit belongs to the persisted v1 contract, not the current tree policy.
const JSON_V1_MAX_DEPTH: u8 = 64;

const TAG_NULL: u8 = 0x01;
const TAG_BOOL: u8 = 0x02;
const TAG_INT: u8 = 0x03;
const TAG_FLOAT: u8 = 0x04;
const TAG_STRING: u8 = 0x05;
const TAG_JSON_ARRAY: u8 = 0x0B;
const TAG_JSON_OBJECT: u8 = 0x0C;

/// Encode JSON using the fixed version-1 format used by persisted plan identities.
///
/// These are exactly the bytes formerly produced by
/// `FieldValue::Literal(value.clone()).canonical_bytes()`: the domain
/// `nbschema-value-v`, a big-endian `u16` version of `1`, then the JSON body.
/// Arrays and objects use tags `0x0B` and `0x0C`, not the typed-tree tags.
/// Object keys are sorted by UTF-8 bytes, without Unicode normalization.
///
/// This encodes data only: strings and expression/mode-shaped objects are never
/// interpreted. Values are not redacted; callers must exclude secret material.
/// Changes to the current tree codec must not change this durable protocol.
///
/// # Errors
///
/// Returns `recursion_limit` if any node exceeds depth 64 (the root is depth 0),
/// or `value.non_canonical_float` if a number cannot be represented as a finite
/// value. The numeric encoding preserves v1's strict `abs(float) < 2^127` bound,
/// including encoding `-2^127` as a float rather than an integer.
///
/// # Examples
///
/// ```
/// use nebula_schema::canonical_json_v1;
///
/// let bytes = canonical_json_v1(&serde_json::json!({"a": 1}))?;
/// assert!(bytes.starts_with(b"nbschema-value-v\x00\x01\x0c"));
/// # Ok::<(), nebula_schema::ValidationError>(())
/// ```
#[must_use = "canonical bytes must be checked before deriving a durable identity"]
#[tracing::instrument(level = "trace", skip_all, fields(canonical_version = JSON_V1_VERSION))]
pub fn canonical_json_v1(value: &Value) -> Result<Vec<u8>, ValidationError> {
    let mut out = Vec::new();
    out.extend_from_slice(JSON_V1_DOMAIN);
    out.extend_from_slice(&JSON_V1_VERSION.to_be_bytes());
    write_json_v1(value, &mut out, 0)?;
    Ok(out)
}

fn non_canonical_float() -> ValidationError {
    ValidationError::builder("value.non_canonical_float")
        .message("non-finite floats (NaN / \u{b1}Inf) have no canonical encoding")
        .build()
}

fn canon_recursion_limit() -> ValidationError {
    ValidationError::builder("recursion_limit")
        .message("value nesting exceeds the maximum canonicalization depth")
        .build()
}

/// Append a length-prefixed byte string, using unsigned LEB128 for the length.
pub(super) fn write_lp(out: &mut Vec<u8>, bytes: &[u8]) {
    write_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

/// Append an unsigned LEB128 varint.
pub(super) fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// Append a JSON-v1 body without its domain or version, continuing the depth bound.
/// The tree writer shares this scalar encoding; its containers use its own tags.
pub(super) fn write_json_v1(
    value: &Value,
    out: &mut Vec<u8>,
    depth: u8,
) -> Result<(), ValidationError> {
    if depth > JSON_V1_MAX_DEPTH {
        return Err(canon_recursion_limit());
    }
    match value {
        Value::Null => out.push(TAG_NULL),
        Value::Bool(boolean) => {
            out.push(TAG_BOOL);
            out.push(u8::from(*boolean));
        },
        Value::Number(number) => write_canon_number(number, out)?,
        Value::String(string) => {
            out.push(TAG_STRING);
            write_lp(out, string.as_bytes());
        },
        Value::Array(items) => {
            out.push(TAG_JSON_ARRAY);
            write_varint(out, items.len() as u64);
            for item in items {
                write_json_v1(item, out, depth.saturating_add(1))?;
            }
        },
        Value::Object(map) => {
            out.push(TAG_JSON_OBJECT);
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_unstable_by(|(a, _), (b, _)| a.as_bytes().cmp(b.as_bytes()));
            write_varint(out, entries.len() as u64);
            for (key, child) in entries {
                write_lp(out, key.as_bytes());
                write_json_v1(child, out, depth.saturating_add(1))?;
            }
        },
    }
    Ok(())
}

/// Integers and in-range integral floats share an i128 big-endian encoding;
/// other finite floats keep their IEEE-754 big-endian bits.
fn write_canon_number(
    number: &serde_json::Number,
    out: &mut Vec<u8>,
) -> Result<(), ValidationError> {
    if let Some(int) = number.as_i64() {
        out.push(TAG_INT);
        out.extend_from_slice(&i128::from(int).to_be_bytes());
        return Ok(());
    }
    if let Some(uint) = number.as_u64() {
        out.push(TAG_INT);
        out.extend_from_slice(&i128::from(uint).to_be_bytes());
        return Ok(());
    }
    let float = number.as_f64().ok_or_else(non_canonical_float)?;
    if !float.is_finite() {
        return Err(non_canonical_float());
    }
    // Preserve the original strict bound, even at the representable -2^127
    // endpoint. Widening it would change existing durable v1 bytes.
    let i128_bound = 2.0_f64.powi(127);
    if float.fract() == 0.0 && float.abs() < i128_bound {
        out.push(TAG_INT);
        // Integral and strictly within i128's range, so this cast is exact.
        let as_int = float as i128;
        out.extend_from_slice(&as_int.to_be_bytes());
        return Ok(());
    }
    out.push(TAG_FLOAT);
    out.extend_from_slice(&float.to_be_bytes());
    Ok(())
}
