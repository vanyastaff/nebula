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
//! sorted keys — except a number inside raw JSON text (a `RawValue`, or a
//! `Number` under `arbitrary_precision`), which keeps its exact decimal
//! value ([`canonical_number`]) instead of collapsing through an `f64`.

use std::{cell::Cell, fmt, io};

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
/// is canonicalized as it is parsed, with the same duplicate-key refusal
/// and cap as any other value.
const RAW_VALUE_TOKEN: &str = "$serde_json::private::RawValue";

/// The serde name a `serde_json::Number` serializes under when a crate in
/// the build enables `serde_json`'s `arbitrary_precision`: its one field is
/// the number's decimal text, written as [`canonical_number`] writes any
/// raw number — so the canonical bytes do not depend on that feature.
const NUMBER_TOKEN: &str = "$serde_json::private::Number";

/// Why a request has no canonical form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CanonicalError {
    /// The request does not serialize to JSON.
    Unserializable,
    /// An object of the request writes one key twice.
    DuplicateKey,
    /// The canonical form outgrows [`MAX_CANONICAL_REQUEST_LEN`].
    TooLarge,
    /// A float is NaN or infinite, refused by [`to_canonical_finite`] only:
    /// JSON writes it `null`, aliasing a real `null`.
    NonFiniteFloat,
}

impl fmt::Display for CanonicalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unserializable => "the request does not serialize to JSON",
            Self::DuplicateKey => "an object of the request has a duplicate key",
            Self::TooLarge => "the canonical request is over its cap",
            Self::NonFiniteFloat => "a float of the value is NaN or infinite",
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

/// The canonical JSON of `value`, refusing a NaN or infinite float
/// ([`CanonicalError::NonFiniteFloat`]) — as a value or a map key — rather
/// than writing it `null`.
///
/// For a digest that must tell every distinct value apart, such as a
/// configuration fingerprint. An operation request keeps
/// [`to_canonical`]: what reaches its provider is `serde_json`'s output,
/// which writes such a float `null` too, so the alias is faithful there.
pub(crate) fn to_canonical_finite<T: Serialize + ?Sized>(
    value: &T,
) -> Result<Vec<u8>, CanonicalError> {
    let mut out = Vec::new();
    let budget = Budget {
        finite_only: true,
        ..Budget::default()
    };
    value.serialize(Canonical {
        out: &mut out,
        budget: &budget,
    })?;
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
    /// Refuse a non-finite float instead of writing it `null`.
    finite_only: bool,
}

impl Budget {
    /// Refuses `finite == false` when only finite floats are admitted.
    fn float(&self, finite: bool) -> Result<(), CanonicalError> {
        if self.finite_only && !finite {
            Err(CanonicalError::NonFiniteFloat)
        } else {
            Ok(())
        }
    }

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
        serialize_char(char);
    }

    non_keys! {
        serialize_bytes(&[u8]);
        serialize_none();
        serialize_unit();
        serialize_unit_struct(&'static str);
    }

    fn serialize_f32(self, key: f32) -> Result<String, CanonicalError> {
        self.budget.float(key.is_finite())?;
        scalar_key(key)
    }

    fn serialize_f64(self, key: f64) -> Result<String, CanonicalError> {
        self.budget.float(key.is_finite())?;
        scalar_key(key)
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

    fn object(
        self,
        close: &'static [u8],
        raw: Option<&'static str>,
    ) -> Result<Object<'a>, CanonicalError> {
        if raw.is_none() {
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
        self.budget.float(value.is_finite())?;
        push_scalar(self.out, self.budget, value)
    }

    fn serialize_f64(self, value: f64) -> Result<(), CanonicalError> {
        self.budget.float(value.is_finite())?;
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
        self.object(b"}", None)
    }

    fn serialize_struct(
        self,
        name: &'static str,
        _len: usize,
    ) -> Result<Object<'a>, CanonicalError> {
        let raw = [RAW_VALUE_TOKEN, NUMBER_TOKEN]
            .into_iter()
            .find(|token| name == *token);
        self.object(b"}", raw)
    }

    fn serialize_struct_variant(
        mut self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Object<'a>, CanonicalError> {
        self.open_variant(variant)?;
        self.object(b"}}", None)
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
    /// A `serde_json` private token struct — `RawValue`, or `Number` under
    /// `arbitrary_precision` — named by its token: its one field is JSON
    /// text, written canonicalized in place of the object.
    raw: Option<&'static str>,
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

    fn raw_text<T: Serialize + ?Sized>(
        &mut self,
        value: &T,
        number_only: bool,
    ) -> Result<(), CanonicalError> {
        value.serialize(RawText {
            out: &mut *self.out,
            budget: self.budget,
            number_only,
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
        if raw.is_some() {
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
        if let Some(token) = self.raw {
            return if key == token {
                self.raw_text(value, token == NUMBER_TOKEN)
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

/// The JSON text of a `RawValue` (or of an `arbitrary_precision`
/// `Number`), the one field it serializes: anything but a string is
/// refused.
struct RawText<'a> {
    out: &'a mut Vec<u8>,
    budget: &'a Budget,
    /// The text must be one JSON number (a `Number`'s).
    number_only: bool,
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
        if self.number_only {
            let number = canonical_number(text)?;
            push(self.out, self.budget, number.as_bytes())
        } else {
            transcode(self.out, self.budget, text)
        }
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
    let mut raw = RawJson { text, at: 0 };
    raw.value(out, budget, 0)?;
    raw.skip_whitespace();
    if raw.at == text.len() {
        Ok(())
    } else {
        Err(CanonicalError::Unserializable)
    }
}

/// Deepest nesting of raw JSON text transcoded, as deep as `serde_json`
/// parses by default.
const MAX_RAW_DEPTH: usize = 128;

/// A raw JSON text read one token at a time, validated as it is read.
///
/// Hand-rolled rather than driven by `serde_json`'s parser so that a
/// number keeps its exact decimal text ([`canonical_number`]): the parser
/// hands a visitor an `f64` for every non-integer (and every integer past
/// `u64`), which would make distinct numbers such as `9007199254740992.0`
/// and `9007199254740993.0` one canonical value, and it hands numbers over
/// differently when a dependency enables `arbitrary_precision`. A string
/// token is still decoded by `serde_json`, so escapes are its own.
struct RawJson<'t> {
    text: &'t str,
    at: usize,
}

impl RawJson<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    /// Consumes `byte` when it is next.
    fn eat(&mut self, byte: u8) -> bool {
        let next = self.peek() == Some(byte);
        if next {
            self.at += 1;
        }
        next
    }

    /// Writes the value at the cursor canonicalized into `out`.
    fn value(
        &mut self,
        out: &mut Vec<u8>,
        budget: &Budget,
        depth: usize,
    ) -> Result<(), CanonicalError> {
        self.skip_whitespace();
        match self.peek() {
            Some(b'{') => self.object(out, budget, depth),
            Some(b'[') => self.array(out, budget, depth),
            Some(b'"') => {
                let text = self.string()?;
                push_str(out, budget, &text)
            },
            Some(b't') => self.literal("true", out, budget),
            Some(b'f') => self.literal("false", out, budget),
            Some(b'n') => self.literal("null", out, budget),
            Some(b'-' | b'0'..=b'9') => {
                let start = self.at;
                while matches!(
                    self.peek(),
                    Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                ) {
                    self.at += 1;
                }
                let text = self
                    .text
                    .get(start..self.at)
                    .ok_or(CanonicalError::Unserializable)?;
                let number = canonical_number(text)?;
                push(out, budget, number.as_bytes())
            },
            _ => Err(CanonicalError::Unserializable),
        }
    }

    fn literal(
        &mut self,
        word: &'static str,
        out: &mut Vec<u8>,
        budget: &Budget,
    ) -> Result<(), CanonicalError> {
        if self
            .text
            .get(self.at..)
            .is_some_and(|rest| rest.starts_with(word))
        {
            self.at += word.len();
            push(out, budget, word.as_bytes())
        } else {
            Err(CanonicalError::Unserializable)
        }
    }

    /// The decoded string token at the cursor.
    fn string(&mut self) -> Result<String, CanonicalError> {
        let bytes = self.text.as_bytes();
        let mut end = self.at + 1;
        loop {
            match bytes.get(end) {
                Some(b'"') => break,
                Some(b'\\') => end += 2,
                Some(_) => end += 1,
                None => return Err(CanonicalError::Unserializable),
            }
        }
        let token = self
            .text
            .get(self.at..=end)
            .ok_or(CanonicalError::Unserializable)?;
        self.at = end + 1;
        serde_json::from_str(token).map_err(|_| CanonicalError::Unserializable)
    }

    fn array(
        &mut self,
        out: &mut Vec<u8>,
        budget: &Budget,
        depth: usize,
    ) -> Result<(), CanonicalError> {
        if depth >= MAX_RAW_DEPTH {
            return Err(CanonicalError::Unserializable);
        }
        self.at += 1;
        push(out, budget, b"[")?;
        self.skip_whitespace();
        if !self.eat(b']') {
            loop {
                self.value(out, budget, depth + 1)?;
                self.skip_whitespace();
                if self.eat(b']') {
                    break;
                }
                if !self.eat(b',') {
                    return Err(CanonicalError::Unserializable);
                }
                // The comma is written only once its element is found.
                self.skip_whitespace();
                if matches!(self.peek(), None | Some(b']')) {
                    return Err(CanonicalError::Unserializable);
                }
                push(out, budget, b",")?;
            }
        }
        push(out, budget, b"]")
    }

    fn object(
        &mut self,
        out: &mut Vec<u8>,
        budget: &Budget,
        depth: usize,
    ) -> Result<(), CanonicalError> {
        if depth >= MAX_RAW_DEPTH {
            return Err(CanonicalError::Unserializable);
        }
        self.at += 1;
        let mut object = Canonical { out, budget }.object(b"}", None)?;
        self.skip_whitespace();
        if !self.eat(b'}') {
            loop {
                self.skip_whitespace();
                if self.peek() != Some(b'"') {
                    return Err(CanonicalError::Unserializable);
                }
                let key = self.string()?;
                // The one text kept apart from the output: refused once it
                // cannot fit as a JSON string.
                check_key(budget, key.len())?;
                self.skip_whitespace();
                if !self.eat(b':') {
                    return Err(CanonicalError::Unserializable);
                }
                let mut member = object.begin_member(&key)?;
                self.value(&mut member, budget, depth + 1)?;
                object.members.push((key, member));
                self.skip_whitespace();
                if self.eat(b'}') {
                    break;
                }
                if !self.eat(b',') {
                    return Err(CanonicalError::Unserializable);
                }
            }
        }
        object.close()
    }
}

/// A JSON number's decimal value, normalized: `digits × 10^exponent`, with
/// neither leading nor trailing zeros in `digits`; zero has no digits and
/// no sign.
#[derive(Debug, PartialEq, Eq)]
struct Decimal {
    negative: bool,
    digits: String,
    exponent: i128,
}

/// The decimal value of a JSON number `text`; refused unless `text` is
/// one, by JSON's grammar.
fn decimal(text: &str) -> Result<Decimal, CanonicalError> {
    let refused = CanonicalError::Unserializable;
    let digits_only =
        |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (mantissa, exponent) = match unsigned.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, Some(exponent)),
        None => (unsigned, None),
    };
    let (integer, fraction) = match mantissa.split_once('.') {
        Some((integer, fraction)) => (integer, Some(fraction)),
        None => (mantissa, None),
    };
    if !digits_only(integer)
        || (integer.len() > 1 && integer.starts_with('0'))
        || fraction.is_some_and(|fraction| !digits_only(fraction))
    {
        return Err(refused);
    }
    let exponent: i128 = match exponent {
        None => 0,
        Some(exponent) => {
            if !digits_only(exponent.strip_prefix(['+', '-']).unwrap_or(exponent)) {
                return Err(refused);
            }
            exponent.parse().map_err(|_| refused)?
        },
    };
    let fraction = fraction.unwrap_or("");
    let shift = i128::try_from(fraction.len()).map_err(|_| refused)?;
    let mut exponent = exponent.checked_sub(shift).ok_or(refused)?;
    let mut digits = format!("{integer}{fraction}");
    while digits.ends_with('0') {
        digits.pop();
        exponent = exponent.checked_add(1).ok_or(refused)?;
    }
    let digits = digits.trim_start_matches('0').to_owned();
    if digits.is_empty() {
        return Ok(Decimal {
            negative: false,
            digits,
            exponent: 0,
        });
    }
    Ok(Decimal {
        negative,
        digits,
        exponent,
    })
}

/// The canonical text of the JSON number `text`, from its exact decimal
/// value — independent of how a parser would have typed it.
///
/// - An integer is written as it is (any size; `-0` as `0`), as
///   `serde_json` writes an integer value.
/// - A number with a fraction or an exponent is written as `serde_json`
///   writes the `f64` it parses to, when that text has exactly the same
///   decimal value — so `1.50` and `1.5e0` write `1.5`, as a serialized
///   `1.5_f64` does.
/// - Otherwise the `f64` would lose digits (`9007199254740993.0`), so the
///   exact value is written as `<digits>e<exponent>`: two numbers share a
///   canonical text only when their decimal values are equal.
fn canonical_number(text: &str) -> Result<String, CanonicalError> {
    let exact = decimal(text)?;
    if !text.contains(['.', 'e', 'E']) {
        // `serde_json` parses `-0` as the float `-0.0`: written as it does.
        return Ok(if exact.digits.is_empty() && text.starts_with('-') {
            "-0.0".to_owned()
        } else {
            text.to_owned()
        });
    }
    if let Ok(float) = text.parse::<f64>()
        && float.is_finite()
        && let Ok(shortest) = serde_json::to_string(&float)
        && decimal(&shortest).is_ok_and(|value| value == exact)
    {
        return Ok(shortest);
    }
    let sign = if exact.negative { "-" } else { "" };
    Ok(format!("{sign}{}e{}", exact.digits, exact.exponent))
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

#[cfg(test)]
mod number_tests {
    use serde::{Serialize, ser::SerializeStruct as _};
    use serde_json::value::RawValue;

    use super::{
        CanonicalError, NUMBER_TOKEN, canonical_number, to_canonical, to_canonical_finite,
    };

    /// Serializes as `serde_json::Number` does when a crate in the build
    /// enables `arbitrary_precision`: a private token struct carrying the
    /// number's text. The feature is off in this workspace; this pins that
    /// the canonical bytes would not change if it were turned on.
    struct PrivateNumber(&'static str);

    impl Serialize for PrivateNumber {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut state = serializer.serialize_struct(NUMBER_TOKEN, 1)?;
            state.serialize_field(NUMBER_TOKEN, self.0)?;
            state.end()
        }
    }

    fn text<T: Serialize + ?Sized>(value: &T) -> String {
        String::from_utf8(to_canonical_finite(value).expect("canonical")).expect("utf8")
    }

    #[test]
    fn a_private_number_token_is_its_canonical_number() {
        for (number, typed) in [
            ("8", text(&8_u64)),
            ("-3", text(&-3_i64)),
            ("1.5", text(&1.5_f64)),
            ("0.1", text(&0.1_f64)),
            ("1e2", text(&100.0_f64)),
            ("18446744073709551616", "18446744073709551616".to_owned()),
            ("9007199254740993.0", "9007199254740993e0".to_owned()),
        ] {
            assert_eq!(text(&PrivateNumber(number)), typed, "{number}");
            let raw = RawValue::from_string(number.to_owned()).expect("raw");
            assert_eq!(text(&raw), typed, "raw {number}");
            assert_eq!(
                to_canonical(&PrivateNumber(number)),
                to_canonical(&raw),
                "request mode {number}"
            );
        }
        // Inside a structure, as `serde_json::Value::Number` sits.
        let nested = std::collections::BTreeMap::from([("n", PrivateNumber("2.50"))]);
        assert_eq!(text(&nested), r#"{"n":2.5}"#);
        // The token carries a number, nothing else.
        for not_a_number in [
            "", "01", "1.", ".5", "1e", "+1", "NaN", "\"1\"", "[1]", "1 ",
        ] {
            assert_eq!(
                to_canonical_finite(&PrivateNumber(not_a_number)),
                Err(CanonicalError::Unserializable),
                "{not_a_number:?}"
            );
        }
    }

    #[test]
    fn distinct_decimals_never_share_a_canonical_number() {
        let pairs = [
            ("9007199254740992.0", "9007199254740993.0"),
            ("0.1", "0.10000000000000000001"),
            ("1e308", "1e309"),
            ("18446744073709551616", "18446744073709551617"),
        ];
        for (left, right) in pairs {
            assert_ne!(
                canonical_number(left),
                canonical_number(right),
                "{left} {right}"
            );
        }
        for (left, right) in [("1.5", "1.50"), ("1.5", "15e-1"), ("100.0", "1E2")] {
            assert_eq!(
                canonical_number(left),
                canonical_number(right),
                "{left} {right}"
            );
        }
        // An exact integer past 2^53 is written as it is, every time.
        assert_eq!(
            canonical_number("9007199254740993"),
            Ok("9007199254740993".to_owned())
        );
    }

    #[test]
    fn raw_json_is_validated_as_it_is_read() {
        for broken in [
            "[1,]",
            "[,1]",
            "{\"a\":1,}",
            "{\"a\" 1}",
            "{ unquoted: 1 }",
            "truthy",
            "nil",
            "[1 2]",
            "\"open",
            "01",
            "-",
            "1.e5",
        ] {
            assert_eq!(
                to_canonical(&UncheckedRaw(broken.to_owned())),
                Err(CanonicalError::Unserializable),
                "{broken}"
            );
        }
        // Nesting is bounded as `serde_json` bounds its parse.
        let deep =
            |depth: usize| UncheckedRaw(format!("{}{}", "[".repeat(depth), "]".repeat(depth)));
        assert!(to_canonical(&deep(128)).is_ok());
        assert_eq!(
            to_canonical(&deep(129)),
            Err(CanonicalError::Unserializable)
        );
    }

    /// Serializes as a `RawValue` does, without its check that the text is
    /// JSON.
    struct UncheckedRaw(String);

    impl Serialize for UncheckedRaw {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            const TOKEN: &str = "$serde_json::private::RawValue";
            let mut state = serializer.serialize_struct(TOKEN, 1)?;
            state.serialize_field(TOKEN, &self.0)?;
            state.end()
        }
    }
}
