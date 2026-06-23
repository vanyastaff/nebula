//! Directional schema newtypes (ADR-0100 C15).
//!
//! A [`ValidSchema`] tagged with its dataflow **polarity** — [`Input`] (what a
//! node consumes) or [`Output`] (what a node produces). The assignability
//! relation takes `(&OutputSchema, &InputSchema)`, so swapping producer and
//! consumer is a *compile* error rather than a silent logic bug: direction is
//! enforced by the type system, not by discipline.
//!
//! The polarity lives on the **schema** (the port), not on every [`Field`] — a
//! field is the same shape whether read or written; only the schema as a whole
//! has a dataflow direction. The newtype is `#[repr(transparent)]` and serde-
//! transparent, so it is zero-cost and does not change the wire format.
//!
//! [`Field`]: crate::Field

use core::marker::PhantomData;

use crate::ValidSchema;

mod sealed {
    /// Seals [`Polarity`](super::Polarity) so only this crate's [`Input`] and
    /// [`Output`] can implement it.
    pub trait Sealed {}
}

/// The dataflow direction of a schema. **Sealed**: only [`Input`] and
/// [`Output`] implement it, so the set of polarities is closed.
pub trait Polarity: sealed::Sealed + 'static {
    /// Lowercase label (`"input"` / `"output"`) for diagnostics.
    const LABEL: &'static str;
}

/// The **consumer** polarity: a node's `Input` schema — the shape it expects to
/// receive. Uninhabited; used only as a compile-time marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {}

/// The **producer** polarity: a node's `Output` schema — the shape it emits.
/// Uninhabited; used only as a compile-time marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {}

impl sealed::Sealed for Input {}
impl Polarity for Input {
    const LABEL: &'static str = "input";
}

impl sealed::Sealed for Output {}
impl Polarity for Output {
    const LABEL: &'static str = "output";
}

/// A [`ValidSchema`] tagged with its dataflow [`Polarity`] `P`.
///
/// `#[repr(transparent)]` over the schema — the polarity is a zero-sized
/// compile-time marker. Prefer the [`InputSchema`] / [`OutputSchema`] aliases.
#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct DirectedSchema<P: Polarity> {
    schema: ValidSchema,
    _polarity: PhantomData<fn() -> P>,
}

/// A node's **input** (consumer) schema — what it expects to receive.
pub type InputSchema = DirectedSchema<Input>;

/// A node's **output** (producer) schema — what it emits.
pub type OutputSchema = DirectedSchema<Output>;

impl<P: Polarity> DirectedSchema<P> {
    /// Tag a [`ValidSchema`] with polarity `P`.
    #[must_use]
    pub fn new(schema: ValidSchema) -> Self {
        Self {
            schema,
            _polarity: PhantomData,
        }
    }

    /// Borrow the underlying schema (drops the polarity tag).
    #[must_use]
    pub fn as_schema(&self) -> &ValidSchema {
        &self.schema
    }

    /// Unwrap into the underlying [`ValidSchema`].
    #[must_use]
    pub fn into_schema(self) -> ValidSchema {
        self.schema
    }

    /// The polarity label (`"input"` / `"output"`), for diagnostics.
    #[must_use]
    pub fn polarity(&self) -> &'static str {
        P::LABEL
    }
}

impl<P: Polarity> From<ValidSchema> for DirectedSchema<P> {
    fn from(schema: ValidSchema) -> Self {
        Self::new(schema)
    }
}

// Two `DirectedSchema`s of the same polarity compare by their underlying schema
// (the polarity is a phantom). Cross-polarity comparison does not type-check.
impl<P: Polarity> PartialEq for DirectedSchema<P> {
    fn eq(&self, other: &Self) -> bool {
        self.schema == other.schema
    }
}

// Serde-transparent: a `DirectedSchema` serializes exactly as its `ValidSchema`,
// so the polarity is compile-time only and the wire format is unchanged.
impl<P: Polarity> serde::Serialize for DirectedSchema<P> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.schema.serialize(serializer)
    }
}

impl<'de, P: Polarity> serde::Deserialize<'de> for DirectedSchema<P> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ValidSchema::deserialize(deserializer).map(Self::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_input() -> InputSchema {
        InputSchema::new(ValidSchema::empty())
    }

    #[test]
    fn wraps_and_unwraps_without_changing_schema() {
        let schema = ValidSchema::empty();
        let directed = OutputSchema::new(schema.clone());
        assert_eq!(directed.as_schema(), &schema);
        assert_eq!(directed.into_schema(), schema);
    }

    #[test]
    fn polarity_label_reflects_type() {
        assert_eq!(empty_input().polarity(), "input");
        assert_eq!(OutputSchema::new(ValidSchema::empty()).polarity(), "output");
    }

    #[test]
    fn serde_is_transparent_over_valid_schema() {
        let directed = OutputSchema::new(ValidSchema::any());
        let directed_json = serde_json::to_string(&directed).unwrap();
        let bare_json = serde_json::to_string(&ValidSchema::any()).unwrap();
        assert_eq!(
            directed_json, bare_json,
            "the polarity tag must not appear on the wire"
        );
        let decoded: OutputSchema = serde_json::from_str(&directed_json).unwrap();
        assert_eq!(decoded, directed);
    }

    #[test]
    fn from_valid_schema_tags_polarity() {
        let _input: InputSchema = ValidSchema::empty().into();
        let _output: OutputSchema = ValidSchema::empty().into();
    }
}
