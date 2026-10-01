//! The serializer behind a canonical request: compact JSON with every
//! object's keys sorted, written straight from the request's `Serialize`
//! impl.
//!
//! Nothing is materialized as a [`Value`] first, so an object that writes
//! one key twice — a `#[serde(flatten)]` collision, a hand-written impl —
//! is refused instead of keeping its last member. Every output byte —
//! punctuation, escaped strings, scalars, formatted text, a raw value's
//! re-emitted JSON, an object member's buffer — goes through one
//! budget-charging [`Sink`] that refuses a write past the cap before the
//! buffer grows; the only text held apart, an object key kept unescaped to
//! sort by, is checked against the budget before it is copied. Scalars,
//! scalar map keys and string escapes go through `serde_json` itself, so
//! the bytes are exactly those of the request's JSON value re-emitted with
//! sorted keys.

use std::{cell::Cell, fmt, io};

use serde::{
    Deserializer, Serialize, Serializer,
    de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor},
    ser::{
        self, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
        SerializeTupleStruct, SerializeTupleVariant,
    },
};
use serde_json::Value;

use super::declaration::MAX_CANONICAL_REQUEST_LEN;

/// The serde name `serde_json`'s `RawValue` serializes under: its JSON text
/// is canonicalized as it is parsed, with the same duplicate-key refusal
/// and cap as any other value.
const RAW_VALUE_TOKEN: &str = "$serde_json::private::RawValue";

/// Why a request has no canonical form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CanonicalError {
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
pub(crate) fn to_canonical<T: Serialize + ?Sized>(request: &T) -> Result<Vec<u8>, CanonicalError> {
    let mut out = Vec::new();
    write_canonical(request, &mut out)?;
    Ok(out)
}

/// Writes the canonical JSON of `request` into `out`; on a refusal, `out`
/// holds what was written up to it, never more than the cap.
fn write_canonical<T: Serialize + ?Sized>(
    request: &T,
    out: &mut Vec<u8>,
) -> Result<(), CanonicalError> {
    let budget = Budget::default();
    request.serialize(Canonical {
        out,
        budget: &budget,
    })
}

/// Bytes of canonical output written so far.
///
/// Every byte of output goes through a [`Sink`], which charges it before
/// it is written: into the output, or into an object member's buffer later
/// moved into place uncharged. So the charge never exceeds the final length
/// and equals it at the end, and nothing past the cap is ever written.
#[derive(Default)]
struct Budget {
    used: Cell<usize>,
    /// Set once a write or a copy was refused for the cap: whatever error
    /// the refusal surfaces as — `serde_json`'s, a formatter's, a parser's —
    /// it is [`CanonicalError::TooLarge`].
    over: Cell<bool>,
}

impl Budget {
    /// Refuses when `len` more bytes would not fit.
    fn check(&self, len: usize) -> Result<(), CanonicalError> {
        if self.used.get().saturating_add(len) > MAX_CANONICAL_REQUEST_LEN {
            self.over.set(true);
            Err(CanonicalError::TooLarge)
        } else {
            Ok(())
        }
    }

    /// Charges `len` bytes about to be written.
    fn charge(&self, len: usize) -> Result<(), CanonicalError> {
        self.check(len)?;
        self.used.set(self.used.get().saturating_add(len));
        Ok(())
    }

    /// What a failed write surfaces as: the cap if it refused, otherwise
    /// a request that does not serialize.
    fn refusal(&self) -> CanonicalError {
        if self.over.get() {
            CanonicalError::TooLarge
        } else {
            CanonicalError::Unserializable
        }
    }
}

/// The one way bytes reach the output or a member's buffer: each write is
/// charged to the budget before the buffer grows, and refused past the cap.
struct Sink<'a> {
    out: &'a mut Vec<u8>,
    budget: &'a Budget,
}

impl io::Write for Sink<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.budget
            .charge(bytes.len())
            .map_err(|_| io::Error::from(io::ErrorKind::Other))?;
        self.out.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Writes `bytes` of punctuation.
fn push(out: &mut Vec<u8>, budget: &Budget, bytes: &[u8]) -> Result<(), CanonicalError> {
    io::Write::write_all(&mut Sink { out, budget }, bytes).map_err(|_| budget.refusal())
}

/// Writes `text` as a JSON string, escaped as `serde_json` escapes it, each
/// run and escape charged as it is written.
fn push_str(out: &mut Vec<u8>, budget: &Budget, text: &str) -> Result<(), CanonicalError> {
    serde_json::to_writer(Sink { out, budget }, text).map_err(|_| budget.refusal())
}

/// Writes the `Display` text of `value` as a JSON string, escaped as it is
/// formatted: formatting stops at the cap.
fn push_display<T: fmt::Display + ?Sized>(
    out: &mut Vec<u8>,
    budget: &Budget,
    value: &T,
) -> Result<(), CanonicalError> {
    let mut writer = serde_json::Serializer::new(Sink { out, budget });
    Serializer::collect_str(&mut writer, value).map_err(|_| budget.refusal())
}

/// Writes a scalar as the JSON value `serde_json` makes of it: an `f32`
/// widened, a non-finite float `null`, a 128-bit integer only in range.
fn push_scalar<T: Serialize>(
    out: &mut Vec<u8>,
    budget: &Budget,
    scalar: T,
) -> Result<(), CanonicalError> {
    let value = serde_json::to_value(scalar).map_err(|_| CanonicalError::Unserializable)?;
    serde_json::to_writer(Sink { out, budget }, &value).map_err(|_| budget.refusal())
}

/// A scalar map key (a number, a bool, a char) as `serde_json` stringifies
/// it, read back from a one-member object. Its text is a few bytes.
fn scalar_key<T: Serialize>(key: T) -> Result<String, CanonicalError> {
    match serde_json::value::Serializer.collect_map(std::iter::once((key, ()))) {
        Ok(Value::Object(map)) => map
            .into_iter()
            .next()
            .map(|(key, _)| key)
            .ok_or(CanonicalError::Unserializable),
        _ => Err(CanonicalError::Unserializable),
    }
}

/// Refuses a key's unescaped text of `len` bytes when it cannot fit as a
/// JSON string: escaping only lengthens it, and the quotes count.
///
/// A key is the one text kept apart from the output — unescaped, to sort
/// its object's members — so it is checked here before it is copied; its
/// escaped form is charged when the member is written. The keys held thus
/// never outgrow the output they head, nor the cap.
fn check_key(budget: &Budget, len: usize) -> Result<(), CanonicalError> {
    budget.check(len.saturating_add(2))
}

/// The `Display` text of a key, collected only while it can still fit as
/// a JSON string: a longer one is refused as it is formatted.
fn display_key<T: fmt::Display + ?Sized>(
    budget: &Budget,
    key: &T,
) -> Result<String, CanonicalError> {
    struct Bounded<'a> {
        text: String,
        budget: &'a Budget,
    }

    impl fmt::Write for Bounded<'_> {
        fn write_str(&mut self, part: &str) -> fmt::Result {
            check_key(self.budget, self.text.len().saturating_add(part.len()))
                .map_err(|_| fmt::Error)?;
            self.text.push_str(part);
            Ok(())
        }
    }

    let mut bounded = Bounded {
        text: String::new(),
        budget,
    };
    fmt::write(&mut bounded, format_args!("{key}")).map_err(|_| budget.refusal())?;
    Ok(bounded.text)
}

/// A map key, stringified as `serde_json` stringifies it — a string, a
/// number, a bool, a char, a unit variant, a newtype of one — and refused
/// before it is copied, or as it is formatted, once it cannot fit.
struct MapKey<'a> {
    budget: &'a Budget,
}

/// Stringifies a scalar key as `serde_json` does.
macro_rules! scalar_keys {
    ($($method:ident($scalar:ty);)*) => {
        $(
            fn $method(self, key: $scalar) -> Result<String, CanonicalError> {
                scalar_key(key)
            }
        )*
    };
}

/// Refuses a key that is not a string, a scalar or a newtype of one.
macro_rules! non_keys {
    ($($method:ident($($arg:ty),*);)*) => {
        $(
            fn $method(self, $(_: $arg),*) -> Result<String, CanonicalError> {
                Err(CanonicalError::Unserializable)
            }
        )*
    };
}

impl Serializer for MapKey<'_> {
    type Ok = String;
    type Error = CanonicalError;
    type SerializeSeq = ser::Impossible<String, CanonicalError>;
    type SerializeTuple = ser::Impossible<String, CanonicalError>;
    type SerializeTupleStruct = ser::Impossible<String, CanonicalError>;
    type SerializeTupleVariant = ser::Impossible<String, CanonicalError>;
    type SerializeMap = ser::Impossible<String, CanonicalError>;
    type SerializeStruct = ser::Impossible<String, CanonicalError>;
    type SerializeStructVariant = ser::Impossible<String, CanonicalError>;

    scalar_keys! {
        serialize_bool(bool);
        serialize_i8(i8);
        serialize_i16(i16);
        serialize_i32(i32);
        serialize_i64(i64);
        serialize_i128(i128);
        serialize_u8(u8);
        serialize_u16(u16);
        serialize_u32(u32);
        serialize_u64(u64);
        serialize_u128(u128);
        serialize_f32(f32);
        serialize_f64(f64);
        serialize_char(char);
    }

    non_keys! {
        serialize_bytes(&[u8]);
        serialize_none();
        serialize_unit();
        serialize_unit_struct(&'static str);
    }

    fn serialize_str(self, key: &str) -> Result<String, CanonicalError> {
        check_key(self.budget, key.len())?;
        Ok(key.to_owned())
    }

    fn collect_str<T: fmt::Display + ?Sized>(self, key: &T) -> Result<String, CanonicalError> {
        display_key(self.budget, key)
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<String, CanonicalError> {
        self.serialize_str(variant)
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        key: &T,
    ) -> Result<String, CanonicalError> {
        key.serialize(self)
    }

    fn serialize_some<T: Serialize + ?Sized>(self, _key: &T) -> Result<String, CanonicalError> {
        Err(CanonicalError::Unserializable)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _key: &T,
    ) -> Result<String, CanonicalError> {
        Err(CanonicalError::Unserializable)
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, CanonicalError> {
        Err(CanonicalError::Unserializable)
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, CanonicalError> {
        Err(CanonicalError::Unserializable)
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, CanonicalError> {
        Err(CanonicalError::Unserializable)
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, CanonicalError> {
        Err(CanonicalError::Unserializable)
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, CanonicalError> {
        Err(CanonicalError::Unserializable)
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, CanonicalError> {
        Err(CanonicalError::Unserializable)
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, CanonicalError> {
        Err(CanonicalError::Unserializable)
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

    fn collect_str<T: fmt::Display + ?Sized>(self, value: &T) -> Result<(), CanonicalError> {
        push_display(self.out, self.budget, value)
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
        let mut member = self.begin_member(&key)?;
        value.serialize(Canonical {
            out: &mut member,
            budget: self.budget,
        })?;
        self.members.push((key, member));
        Ok(())
    }

    /// A member's buffer holding `"key":`, its value still to be written.
    fn begin_member(&self, key: &str) -> Result<Vec<u8>, CanonicalError> {
        if !self.members.is_empty() {
            // The comma that separates this member from another, wherever
            // the sort puts it.
            self.budget.charge(1)?;
        }
        let mut member = Vec::new();
        push_str(&mut member, self.budget, key)?;
        push(&mut member, self.budget, b":")?;
        Ok(member)
    }

    fn raw_text<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), CanonicalError> {
        value.serialize(RawText {
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
        self.key = Some(key.serialize(MapKey {
            budget: self.budget,
        })?);
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

/// The JSON text of a `RawValue`, the one field it serializes: anything
/// but a string is refused.
struct RawText<'a> {
    out: &'a mut Vec<u8>,
    budget: &'a Budget,
}

/// Refuses every scalar but the raw text's string.
macro_rules! refuse_non_text {
    ($($method:ident($($arg:ty),*);)*) => {
        $(
            fn $method(self, $(_: $arg),*) -> Result<(), CanonicalError> {
                Err(CanonicalError::Unserializable)
            }
        )*
    };
}

impl Serializer for RawText<'_> {
    type Ok = ();
    type Error = CanonicalError;
    type SerializeSeq = ser::Impossible<(), CanonicalError>;
    type SerializeTuple = ser::Impossible<(), CanonicalError>;
    type SerializeTupleStruct = ser::Impossible<(), CanonicalError>;
    type SerializeTupleVariant = ser::Impossible<(), CanonicalError>;
    type SerializeMap = ser::Impossible<(), CanonicalError>;
    type SerializeStruct = ser::Impossible<(), CanonicalError>;
    type SerializeStructVariant = ser::Impossible<(), CanonicalError>;

    refuse_non_text! {
        serialize_bool(bool);
        serialize_i8(i8);
        serialize_i16(i16);
        serialize_i32(i32);
        serialize_i64(i64);
        serialize_u8(u8);
        serialize_u16(u16);
        serialize_u32(u32);
        serialize_u64(u64);
        serialize_f32(f32);
        serialize_f64(f64);
        serialize_char(char);
        serialize_bytes(&[u8]);
        serialize_none();
        serialize_unit();
        serialize_unit_struct(&'static str);
        serialize_unit_variant(&'static str, u32, &'static str);
    }

    fn serialize_str(self, text: &str) -> Result<(), CanonicalError> {
        transcode(self.out, self.budget, text)
    }

    fn serialize_some<T: Serialize + ?Sized>(self, _value: &T) -> Result<(), CanonicalError> {
        Err(CanonicalError::Unserializable)
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _value: &T,
    ) -> Result<(), CanonicalError> {
        Err(CanonicalError::Unserializable)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<(), CanonicalError> {
        Err(CanonicalError::Unserializable)
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, CanonicalError> {
        Err(CanonicalError::Unserializable)
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, CanonicalError> {
        Err(CanonicalError::Unserializable)
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, CanonicalError> {
        Err(CanonicalError::Unserializable)
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, CanonicalError> {
        Err(CanonicalError::Unserializable)
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, CanonicalError> {
        Err(CanonicalError::Unserializable)
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, CanonicalError> {
        Err(CanonicalError::Unserializable)
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, CanonicalError> {
        Err(CanonicalError::Unserializable)
    }
}

/// Writes the JSON `text` canonicalized, as it is parsed.
///
/// Never materialized as a [`Value`]: an object with a duplicate key is
/// refused rather than keeping its last member, and the parse stops as
/// soon as the output crosses the cap. Scalars and strings reach the
/// writers a serialized value does, so the bytes are those of the parsed
/// value re-emitted with sorted keys.
fn transcode(out: &mut Vec<u8>, budget: &Budget, text: &str) -> Result<(), CanonicalError> {
    let failure = Cell::new(None);
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let written = Transcode {
        out,
        budget,
        failure: &failure,
    }
    .deserialize(&mut deserializer)
    .and_then(|()| deserializer.end());
    // The parser's own message may echo request text: only the writer's
    // refusal or the cap's survives, any other failure is unserializable.
    written.map_err(|_| failure.take().unwrap_or_else(|| budget.refusal()))
}

/// A raw object's key, copied out of the parser only once it can fit.
struct RawKey<'a> {
    budget: &'a Budget,
}

impl<'de> DeserializeSeed<'de> for RawKey<'_> {
    type Value = String;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<String, D::Error> {
        deserializer.deserialize_str(self)
    }
}

impl Visitor<'_> for RawKey<'_> {
    type Value = String;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an object key")
    }

    fn visit_str<E: de::Error>(self, key: &str) -> Result<String, E> {
        check_key(self.budget, key.len()).map_err(E::custom)?;
        Ok(key.to_owned())
    }
}

/// One JSON value of a raw text, written canonicalized into `out` as the
/// parser reads it.
struct Transcode<'a> {
    out: &'a mut Vec<u8>,
    budget: &'a Budget,
    /// The writer's refusal, carried past the parser's error type.
    failure: &'a Cell<Option<CanonicalError>>,
}

/// Records the writer's refusal for [`transcode`] and stops the parse.
fn refuse<E: de::Error>(failure: &Cell<Option<CanonicalError>>, error: CanonicalError) -> E {
    failure.set(Some(error));
    E::custom(error)
}

impl Transcode<'_> {
    /// Writes with `write`, a refusal recorded for [`transcode`].
    fn write<E: de::Error>(
        &mut self,
        write: impl FnOnce(&mut Vec<u8>, &Budget) -> Result<(), CanonicalError>,
    ) -> Result<(), E> {
        write(self.out, self.budget).map_err(|error| refuse(self.failure, error))
    }
}

impl<'de> DeserializeSeed<'de> for Transcode<'_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

/// An array element: its separating comma is written only once the parser
/// has found the element.
struct Element<'a> {
    value: Transcode<'a>,
    comma: bool,
}

impl<'de> DeserializeSeed<'de> for Element<'_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        let Element { mut value, comma } = self;
        if comma {
            value.write(|out, budget| push(out, budget, b","))?;
        }
        deserializer.deserialize_any(value)
    }
}

impl<'de> Visitor<'de> for Transcode<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E: de::Error>(mut self, value: bool) -> Result<(), E> {
        self.write(|out, budget| push_scalar(out, budget, value))
    }

    fn visit_i64<E: de::Error>(mut self, value: i64) -> Result<(), E> {
        self.write(|out, budget| push_scalar(out, budget, value))
    }

    fn visit_u64<E: de::Error>(mut self, value: u64) -> Result<(), E> {
        self.write(|out, budget| push_scalar(out, budget, value))
    }

    fn visit_f64<E: de::Error>(mut self, value: f64) -> Result<(), E> {
        self.write(|out, budget| push_scalar(out, budget, value))
    }

    fn visit_str<E: de::Error>(mut self, value: &str) -> Result<(), E> {
        self.write(|out, budget| push_str(out, budget, value))
    }

    fn visit_unit<E: de::Error>(mut self) -> Result<(), E> {
        self.write(|out, budget| push(out, budget, b"null"))
    }

    fn visit_seq<A: SeqAccess<'de>>(mut self, mut items: A) -> Result<(), A::Error> {
        self.write(|out, budget| push(out, budget, b"["))?;
        let mut comma = false;
        while items
            .next_element_seed(Element {
                value: Transcode {
                    out: &mut *self.out,
                    budget: self.budget,
                    failure: self.failure,
                },
                comma,
            })?
            .is_some()
        {
            comma = true;
        }
        self.write(|out, budget| push(out, budget, b"]"))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<(), A::Error> {
        let Transcode {
            out,
            budget,
            failure,
        } = self;
        let mut object = Canonical { out, budget }
            .object(b"}", false)
            .map_err(|error| refuse::<A::Error>(failure, error))?;
        while let Some(key) = entries.next_key_seed(RawKey { budget })? {
            let mut member = object
                .begin_member(&key)
                .map_err(|error| refuse::<A::Error>(failure, error))?;
            entries.next_value_seed(Transcode {
                out: &mut member,
                budget,
                failure,
            })?;
            object.members.push((key, member));
        }
        object
            .close()
            .map_err(|error| refuse::<A::Error>(failure, error))
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fmt};

    use serde::Serialize;
    use serde_json::value::RawValue;

    use super::{CanonicalError, MAX_CANONICAL_REQUEST_LEN, RAW_VALUE_TOKEN, write_canonical};

    /// Half the cap of NULs: within the cap as text, six times it escaped.
    fn nuls() -> String {
        "\u{0}".repeat(MAX_CANONICAL_REQUEST_LEN / 2)
    }

    /// Writes `request`, asserting it is refused for the cap with the
    /// output never grown past it.
    fn refused_within_the_cap<T: Serialize + ?Sized>(label: &str, request: &T) {
        let mut out = Vec::new();
        assert_eq!(
            write_canonical(request, &mut out),
            Err(CanonicalError::TooLarge),
            "{label}"
        );
        assert!(
            out.len() <= MAX_CANONICAL_REQUEST_LEN,
            "{label}: {} bytes written",
            out.len()
        );
        assert!(
            out.capacity() <= 2 * MAX_CANONICAL_REQUEST_LEN,
            "{label}: {} bytes reserved",
            out.capacity()
        );
    }

    /// `Display`s its text as many times as it says.
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    struct Repeated(&'static str, usize);

    impl fmt::Display for Repeated {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            for _ in 0..self.1 {
                formatter.write_str(self.0)?;
            }
            Ok(())
        }
    }

    impl Serialize for Repeated {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.collect_str(self)
        }
    }

    /// Serializes as `serde_json`'s `RawValue` does, unchecked.
    struct UncheckedRaw(String);

    impl Serialize for UncheckedRaw {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeStruct as _;
            let mut state = serializer.serialize_struct(RAW_VALUE_TOKEN, 1)?;
            state.serialize_field(RAW_VALUE_TOKEN, self.0.as_str())?;
            state.end()
        }
    }

    #[test]
    fn escapes_are_charged_as_they_are_written() {
        let nuls = nuls();
        refused_within_the_cap("string", nuls.as_str());
        refused_within_the_cap(
            "displayed",
            &Repeated("\u{0}", MAX_CANONICAL_REQUEST_LEN / 2),
        );
        let escaped = format!("\"{}\"", "\\u0000".repeat(MAX_CANONICAL_REQUEST_LEN / 2));
        let raw = RawValue::from_string(escaped).expect("raw");
        refused_within_the_cap("raw string", &raw);
        // In a member's buffer, which the output never sees.
        refused_within_the_cap("member", &BTreeMap::from([("k", nuls.as_str())]));
        refused_within_the_cap("key", &BTreeMap::from([(nuls.as_str(), 1)]));
        refused_within_the_cap(
            "displayed key",
            &BTreeMap::from([(Repeated("\u{0}", MAX_CANONICAL_REQUEST_LEN / 2), 1)]),
        );
    }

    #[test]
    fn a_raw_key_is_refused_before_its_value_is_read() {
        let key = "k".repeat(MAX_CANONICAL_REQUEST_LEN);
        let raw = RawValue::from_string(format!("{{\"{key}\":1}}")).expect("raw");
        refused_within_the_cap("raw key", &raw);
        // The value after the key is never parsed: its broken text is not
        // what refuses the request.
        refused_within_the_cap(
            "raw key, broken value",
            &UncheckedRaw(format!("{{\"{key}\": ]")),
        );
        // A key that fits is kept, in order.
        let mut out = Vec::new();
        let raw = RawValue::from_string(r#"{"b":1,"\u0000":2}"#.to_owned()).expect("raw");
        write_canonical(&raw, &mut out).expect("fits");
        assert_eq!(out, br#"{"\u0000":2,"b":1}"#);
    }
}
