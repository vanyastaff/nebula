//! The serializer behind a canonical request: compact JSON with every
//! object's keys sorted, written straight from the request's `Serialize`
//! impl.
//!
//! Nothing is materialized as a [`Value`] first, so an object that writes
//! one key twice — a `#[serde(flatten)]` collision, a hand-written impl —
//! is refused instead of keeping its last member, and output past the cap
//! stops the serialization as soon as it is written. Scalars and map keys
//! go through `serde_json` itself, so the bytes are exactly those of the
//! request's JSON value re-emitted with sorted keys.

use std::{cell::Cell, fmt};

use serde::{
    Serialize, Serializer,
    ser::{
        self, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
        SerializeTupleStruct, SerializeTupleVariant,
    },
};
use serde_json::Value;

use super::declaration::MAX_CANONICAL_REQUEST_LEN;

/// The serde name `serde_json`'s `RawValue` serializes under: its JSON text
/// is parsed and canonicalized like any other value.
const RAW_VALUE_TOKEN: &str = "$serde_json::private::RawValue";

/// Why a request has no canonical form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CanonicalError {
    /// The request does not serialize to JSON.
    Unserializable,
    /// An object of the request writes one key twice.
    DuplicateKey,
    /// The canonical form outgrows [`MAX_CANONICAL_REQUEST_LEN`].
    TooLarge,
}

impl fmt::Display for CanonicalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unserializable => "the request does not serialize to JSON",
            Self::DuplicateKey => "an object of the request has a duplicate key",
            Self::TooLarge => "the canonical request is over its cap",
        })
    }
}

impl std::error::Error for CanonicalError {}

impl ser::Error for CanonicalError {
    /// A serializer's own message may echo request data: dropped.
    fn custom<T: fmt::Display>(_message: T) -> Self {
        Self::Unserializable
    }
}

/// The canonical JSON of `request`.
pub(super) fn to_canonical<T: Serialize + ?Sized>(request: &T) -> Result<Vec<u8>, CanonicalError> {
    let budget = Budget(Cell::new(0));
    let mut out = Vec::new();
    request.serialize(Canonical {
        out: &mut out,
        budget: &budget,
    })?;
    Ok(out)
}

/// Bytes of canonical output written so far.
///
/// Every byte is charged once, where it is first written — into the output
/// or into an object member's buffer, later moved into place uncharged — so
/// the charge never exceeds the final length and equals it at the end.
struct Budget(Cell<usize>);

impl Budget {
    /// Refuses when `len` more bytes would not fit.
    fn check(&self, len: usize) -> Result<(), CanonicalError> {
        if self.0.get().saturating_add(len) > MAX_CANONICAL_REQUEST_LEN {
            Err(CanonicalError::TooLarge)
        } else {
            Ok(())
        }
    }

    /// Charges `len` bytes written.
    fn charge(&self, len: usize) -> Result<(), CanonicalError> {
        self.check(len)?;
        self.0.set(self.0.get().saturating_add(len));
        Ok(())
    }
}

/// Writes `bytes` of punctuation.
fn push(out: &mut Vec<u8>, budget: &Budget, bytes: &[u8]) -> Result<(), CanonicalError> {
    budget.charge(bytes.len())?;
    out.extend_from_slice(bytes);
    Ok(())
}

/// Writes `text` as a JSON string, escaped as `serde_json` escapes it.
fn push_str(out: &mut Vec<u8>, budget: &Budget, text: &str) -> Result<(), CanonicalError> {
    // Escaping only lengthens a string: one that cannot fit is refused
    // before it is copied.
    budget.check(text.len().saturating_add(2))?;
    let start = out.len();
    serde_json::to_writer(&mut *out, text).map_err(|_| CanonicalError::Unserializable)?;
    budget.charge(out.len().saturating_sub(start))
}

/// Writes a scalar as the JSON value `serde_json` makes of it: an `f32`
/// widened, a non-finite float `null`, a 128-bit integer only in range.
fn push_scalar<T: Serialize>(
    out: &mut Vec<u8>,
    budget: &Budget,
    scalar: T,
) -> Result<(), CanonicalError> {
    let value = serde_json::to_value(scalar).map_err(|_| CanonicalError::Unserializable)?;
    let start = out.len();
    serde_json::to_writer(&mut *out, &value).map_err(|_| CanonicalError::Unserializable)?;
    budget.charge(out.len().saturating_sub(start))
}

/// A map key as `serde_json` stringifies it (a string, a number, a bool, a
/// unit variant), read back from a one-member object.
fn map_key<T: Serialize + ?Sized>(key: &T) -> Result<String, CanonicalError> {
    match serde_json::value::Serializer.collect_map(std::iter::once((key, ()))) {
        Ok(Value::Object(map)) => map
            .into_iter()
            .next()
            .map(|(key, _)| key)
            .ok_or(CanonicalError::Unserializable),
        _ => Err(CanonicalError::Unserializable),
    }
}

/// The canonical serializer, writing one value into `out`.
struct Canonical<'a> {
    out: &'a mut Vec<u8>,
    budget: &'a Budget,
}

impl<'a> Canonical<'a> {
    fn seq(self, open: &[u8], close: &'static [u8]) -> Result<Seq<'a>, CanonicalError> {
        push(self.out, self.budget, open)?;
        Ok(Seq {
            out: self.out,
            budget: self.budget,
            first: true,
            close,
        })
    }

    fn object(self, close: &'static [u8], raw: bool) -> Result<Object<'a>, CanonicalError> {
        if !raw {
            push(self.out, self.budget, b"{")?;
        }
        Ok(Object {
            out: self.out,
            budget: self.budget,
            members: Vec::new(),
            key: None,
            close,
            raw,
        })
    }

    /// Writes `{"variant":`, the opening of an enum variant's object.
    fn open_variant(&mut self, variant: &str) -> Result<(), CanonicalError> {
        push(self.out, self.budget, b"{")?;
        push_str(self.out, self.budget, variant)?;
        push(self.out, self.budget, b":")
    }
}

impl<'a> Serializer for Canonical<'a> {
    type Ok = ();
    type Error = CanonicalError;
    type SerializeSeq = Seq<'a>;
    type SerializeTuple = Seq<'a>;
    type SerializeTupleStruct = Seq<'a>;
    type SerializeTupleVariant = Seq<'a>;
    type SerializeMap = Object<'a>;
    type SerializeStruct = Object<'a>;
    type SerializeStructVariant = Object<'a>;

    fn serialize_bool(self, value: bool) -> Result<(), CanonicalError> {
        push_scalar(self.out, self.budget, value)
    }

    fn serialize_i8(self, value: i8) -> Result<(), CanonicalError> {
        push_scalar(self.out, self.budget, value)
    }

    fn serialize_i16(self, value: i16) -> Result<(), CanonicalError> {
        push_scalar(self.out, self.budget, value)
    }

    fn serialize_i32(self, value: i32) -> Result<(), CanonicalError> {
        push_scalar(self.out, self.budget, value)
    }

    fn serialize_i64(self, value: i64) -> Result<(), CanonicalError> {
        push_scalar(self.out, self.budget, value)
    }

    fn serialize_i128(self, value: i128) -> Result<(), CanonicalError> {
        push_scalar(self.out, self.budget, value)
    }

    fn serialize_u8(self, value: u8) -> Result<(), CanonicalError> {
        push_scalar(self.out, self.budget, value)
    }

    fn serialize_u16(self, value: u16) -> Result<(), CanonicalError> {
        push_scalar(self.out, self.budget, value)
    }

    fn serialize_u32(self, value: u32) -> Result<(), CanonicalError> {
        push_scalar(self.out, self.budget, value)
    }

    fn serialize_u64(self, value: u64) -> Result<(), CanonicalError> {
        push_scalar(self.out, self.budget, value)
    }

    fn serialize_u128(self, value: u128) -> Result<(), CanonicalError> {
        push_scalar(self.out, self.budget, value)
    }

    fn serialize_f32(self, value: f32) -> Result<(), CanonicalError> {
        push_scalar(self.out, self.budget, value)
    }

    fn serialize_f64(self, value: f64) -> Result<(), CanonicalError> {
        push_scalar(self.out, self.budget, value)
    }

    fn serialize_char(self, value: char) -> Result<(), CanonicalError> {
        push_str(self.out, self.budget, value.encode_utf8(&mut [0; 4]))
    }

    fn serialize_str(self, value: &str) -> Result<(), CanonicalError> {
        push_str(self.out, self.budget, value)
    }

    fn serialize_bytes(self, value: &[u8]) -> Result<(), CanonicalError> {
        let mut seq = self.serialize_seq(Some(value.len()))?;
        for byte in value {
            seq.element(byte)?;
        }
        seq.close()
    }

    fn serialize_none(self) -> Result<(), CanonicalError> {
        push(self.out, self.budget, b"null")
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<(), CanonicalError> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<(), CanonicalError> {
        push(self.out, self.budget, b"null")
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<(), CanonicalError> {
        push(self.out, self.budget, b"null")
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<(), CanonicalError> {
        push_str(self.out, self.budget, variant)
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<(), CanonicalError> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        mut self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<(), CanonicalError> {
        self.open_variant(variant)?;
        value.serialize(Canonical {
            out: &mut *self.out,
            budget: self.budget,
        })?;
        push(self.out, self.budget, b"}")
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Seq<'a>, CanonicalError> {
        self.seq(b"[", b"]")
    }

    fn serialize_tuple(self, _len: usize) -> Result<Seq<'a>, CanonicalError> {
        self.seq(b"[", b"]")
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Seq<'a>, CanonicalError> {
        self.seq(b"[", b"]")
    }

    fn serialize_tuple_variant(
        mut self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Seq<'a>, CanonicalError> {
        self.open_variant(variant)?;
        self.seq(b"[", b"]}")
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Object<'a>, CanonicalError> {
        self.object(b"}", false)
    }

    fn serialize_struct(
        self,
        name: &'static str,
        _len: usize,
    ) -> Result<Object<'a>, CanonicalError> {
        self.object(b"}", name == RAW_VALUE_TOKEN)
    }

    fn serialize_struct_variant(
        mut self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Object<'a>, CanonicalError> {
        self.open_variant(variant)?;
        self.object(b"}}", false)
    }
}

/// An array being written: its elements in order.
struct Seq<'a> {
    out: &'a mut Vec<u8>,
    budget: &'a Budget,
    first: bool,
    close: &'static [u8],
}

impl Seq<'_> {
    fn element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), CanonicalError> {
        if !self.first {
            push(self.out, self.budget, b",")?;
        }
        self.first = false;
        value.serialize(Canonical {
            out: &mut *self.out,
            budget: self.budget,
        })
    }

    fn close(self) -> Result<(), CanonicalError> {
        push(self.out, self.budget, self.close)
    }
}

impl SerializeSeq for Seq<'_> {
    type Ok = ();
    type Error = CanonicalError;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
        self.element(value)
    }

    fn end(self) -> Result<(), CanonicalError> {
        self.close()
    }
}

impl SerializeTuple for Seq<'_> {
    type Ok = ();
    type Error = CanonicalError;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
        self.element(value)
    }

    fn end(self) -> Result<(), CanonicalError> {
        self.close()
    }
}

impl SerializeTupleStruct for Seq<'_> {
    type Ok = ();
    type Error = CanonicalError;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
        self.element(value)
    }

    fn end(self) -> Result<(), CanonicalError> {
        self.close()
    }
}

impl SerializeTupleVariant for Seq<'_> {
    type Ok = ();
    type Error = CanonicalError;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
        self.element(value)
    }

    fn end(self) -> Result<(), CanonicalError> {
        self.close()
    }
}

/// An object being written: its members buffered apart, each as
/// `"key":value`, until it ends sorted by key.
struct Object<'a> {
    out: &'a mut Vec<u8>,
    budget: &'a Budget,
    members: Vec<(String, Vec<u8>)>,
    /// A map's key waiting for its value.
    key: Option<String>,
    close: &'static [u8],
    /// A `RawValue`: its one field is JSON text, written canonicalized in
    /// place of the object.
    raw: bool,
}

impl Object<'_> {
    fn member<T: Serialize + ?Sized>(
        &mut self,
        key: String,
        value: &T,
    ) -> Result<(), CanonicalError> {
        if !self.members.is_empty() {
            // The comma that separates this member from another, wherever
            // the sort puts it.
            self.budget.charge(1)?;
        }
        let mut member = Vec::new();
        push_str(&mut member, self.budget, &key)?;
        push(&mut member, self.budget, b":")?;
        value.serialize(Canonical {
            out: &mut member,
            budget: self.budget,
        })?;
        self.members.push((key, member));
        Ok(())
    }

    fn raw_text<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), CanonicalError> {
        let Ok(Value::String(text)) = serde_json::to_value(value) else {
            return Err(CanonicalError::Unserializable);
        };
        let parsed: Value =
            serde_json::from_str(&text).map_err(|_| CanonicalError::Unserializable)?;
        parsed.serialize(Canonical {
            out: &mut *self.out,
            budget: self.budget,
        })
    }

    fn close(self) -> Result<(), CanonicalError> {
        let Object {
            out,
            budget,
            mut members,
            close,
            raw,
            ..
        } = self;
        if raw {
            return Ok(());
        }
        members.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
        if members
            .windows(2)
            .any(|pair| matches!(pair, [(left, _), (right, _)] if left == right))
        {
            return Err(CanonicalError::DuplicateKey);
        }
        for (index, (_, member)) in members.iter().enumerate() {
            if index > 0 {
                // Charged when the member was written.
                out.push(b',');
            }
            out.extend_from_slice(member);
        }
        push(out, budget, close)
    }
}

impl SerializeMap for Object<'_> {
    type Ok = ();
    type Error = CanonicalError;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Self::Error> {
        self.key = Some(map_key(key)?);
        Ok(())
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
        let Some(key) = self.key.take() else {
            return Err(CanonicalError::Unserializable);
        };
        self.member(key, value)
    }

    fn end(self) -> Result<(), CanonicalError> {
        self.close()
    }
}

impl SerializeStruct for Object<'_> {
    type Ok = ();
    type Error = CanonicalError;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        if self.raw {
            return if key == RAW_VALUE_TOKEN {
                self.raw_text(value)
            } else {
                Err(CanonicalError::Unserializable)
            };
        }
        self.member(key.to_owned(), value)
    }

    fn end(self) -> Result<(), CanonicalError> {
        self.close()
    }
}

impl SerializeStructVariant for Object<'_> {
    type Ok = ();
    type Error = CanonicalError;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.member(key.to_owned(), value)
    }

    fn end(self) -> Result<(), CanonicalError> {
        self.close()
    }
}
