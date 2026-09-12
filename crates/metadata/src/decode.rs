//! Transport-bounded JSON ingress for shared and composed recorded metadata.

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{DeserializeOwned, Error as _, MapAccess, Visitor, value::MapAccessDeserializer},
};
use std::{fmt, io::Read, marker::PhantomData};

use crate::MetadataError;

/// Required shared record format, independently of the integration catalog protocol.
pub const METADATA_WIRE_VERSION: u32 = 2;
/// Maximum raw JSON envelope, including schemas, whitespace, and leaf fields.
pub const MAX_METADATA_JSON_BYTES: usize = 4 * 1024 * 1024;
/// Maximum canonical JSON size of a bound input schema.
pub const MAX_METADATA_SCHEMA_BYTES: usize = crate::bounded::SCHEMA_BYTES;
/// Maximum canonical JSON size of shared authored fields, excluding schemas.
pub const MAX_SHARED_METADATA_BYTES: usize = crate::bounded::SHARED_BYTES;

/// Check a complete shared or leaf record against the default JSON envelope.
///
/// Admission and recorded ingress must call this on the whole record, including
/// nested `base`, schemas, and leaf fields. The exact serialized representation
/// is captured within the fixed 4 MiB ceiling and parsed with serde_json's
/// unchanged recursion limit. Shared authored fields retain separate streaming
/// accounting. This checks JSON representability, not domain field validation.
///
/// # Errors
/// Returns payload-free byte, serialization, or JSON shape/depth failures.
#[doc(hidden)]
#[tracing::instrument(name = "metadata.check_json_record", skip_all, err)]
pub fn check_json_record<T: Serialize + ?Sized>(value: &T) -> Result<(), MetadataError> {
    let bytes = crate::bounded::capture_serialized(
        value,
        MAX_METADATA_JSON_BYTES,
        MetadataError::RecordTooLarge,
    )?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| MetadataError::RecordNotDecodable)?;
    if !value.is_object() {
        return Err(MetadataError::RecordNotDecodable);
    }
    Ok(())
}

/// Decode a private strict DTO from an object, never a positional sequence.
///
/// All private visitor diagnostics are discarded before crossing the boundary.
/// The caller remains responsible for field and complete-record validation.
///
/// # Errors
/// Returns only a sanitized invalid-wire diagnostic on structural failure.
#[doc(hidden)]
#[tracing::instrument(name = "metadata.deserialize_object", skip_all)]
pub fn deserialize_metadata_object<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct ObjectVisitor<T>(PhantomData<T>);

    impl<'de, T: Deserialize<'de>> Visitor<'de> for ObjectVisitor<T> {
        type Value = T;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a metadata object")
        }

        fn visit_map<A: MapAccess<'de>>(self, fields: A) -> Result<T, A::Error> {
            T::deserialize(MapAccessDeserializer::new(fields))
        }
    }

    deserializer
        .deserialize_map(ObjectVisitor(PhantomData))
        .map_err(|_| D::Error::custom(MetadataError::InvalidWire))
}

/// A host-selected raw JSON limit which can only lower the protocol ceiling.
///
/// Shared field and schema budgets also apply during recorded deserialization.
/// This limit includes the whole leaf envelope, not just its nested `base`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataDecodeLimits {
    max_envelope_bytes: usize,
}

impl Default for MetadataDecodeLimits {
    fn default() -> Self {
        Self {
            max_envelope_bytes: MAX_METADATA_JSON_BYTES,
        }
    }
}

impl MetadataDecodeLimits {
    /// Select a positive raw envelope limit no greater than 4 MiB.
    ///
    /// # Errors
    /// Returns [`MetadataDecodeError::InvalidLimits`] for zero or a raised ceiling.
    pub fn new(max_envelope_bytes: usize) -> Result<Self, MetadataDecodeError> {
        if max_envelope_bytes == 0 || max_envelope_bytes > MAX_METADATA_JSON_BYTES {
            return Err(MetadataDecodeError::InvalidLimits);
        }
        Ok(Self { max_envelope_bytes })
    }

    /// Maximum bytes accepted before JSON parsing starts.
    #[must_use]
    pub fn max_envelope_bytes(self) -> usize {
        self.max_envelope_bytes
    }
}

/// Payload-free failures from bounded metadata transport decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum MetadataDecodeError {
    /// A host attempted to disable or increase the raw envelope ceiling.
    #[classify(category = "validation", code = "METADATA:DECODE_LIMITS")]
    #[error("invalid metadata decode limits")]
    InvalidLimits,
    /// The raw input exceeds the selected envelope budget.
    #[classify(category = "validation", code = "METADATA:ENVELOPE_BUDGET")]
    #[error("metadata JSON envelope exceeds its byte budget")]
    EnvelopeTooLarge,
    /// JSON syntax, wire shape, version, or intrinsic fields were rejected.
    #[classify(category = "validation", code = "METADATA:DECODE_RECORD")]
    #[error("invalid metadata JSON record")]
    InvalidRecord,
    /// Reading failed; the original I/O diagnostic is not retained.
    #[classify(category = "validation", code = "METADATA:DECODE_IO")]
    #[error("metadata JSON read failed")]
    ReadFailed,
}

/// Decode a recorded shared or leaf DTO after checking the whole raw envelope.
///
/// `T` must be a structurally checked recorded DTO (or `PluginManifest`); its
/// `Deserialize` implementation owns field validation. Generic serde visitors
/// alone cannot bound parser scratch allocation: direct serde callers must
/// provide external transport limits. Owned JSON values are already allocated.
///
/// # Errors
/// Returns a payload-free size or record error, discarding all parser sources.
#[tracing::instrument(name = "metadata.decode_json_slice", skip_all, err)]
pub fn decode_json_slice<T: DeserializeOwned>(
    bytes: &[u8],
    limits: MetadataDecodeLimits,
) -> Result<T, MetadataDecodeError> {
    if bytes.len() > limits.max_envelope_bytes {
        return Err(MetadataDecodeError::EnvelopeTooLarge);
    }
    serde_json::from_slice(bytes).map_err(|_| MetadataDecodeError::InvalidRecord)
}

/// Read at most the envelope limit plus one overflow sentinel before parsing.
///
/// Oversized or endless readers are rejected without JSON parser allocation.
/// Hosts own read deadlines and framing; a reader that never returns can block.
///
/// # Errors
/// Returns sanitized I/O, envelope-size, or record errors without source payloads.
#[tracing::instrument(name = "metadata.decode_json_reader", skip_all, err)]
pub fn decode_json_reader<T: DeserializeOwned>(
    reader: impl Read,
    limits: MetadataDecodeLimits,
) -> Result<T, MetadataDecodeError> {
    let sentinel_limit = u64::try_from(limits.max_envelope_bytes + 1)
        .map_err(|_| MetadataDecodeError::InvalidLimits)?;
    let mut bytes = Vec::new();
    reader
        .take(sentinel_limit)
        .read_to_end(&mut bytes)
        .map_err(|_| MetadataDecodeError::ReadFailed)?;
    decode_json_slice(&bytes, limits)
}
