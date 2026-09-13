//! Version-2 authored wire format, separate from JSON views and canonical hashes.
//!
//! The exact envelope is `{"version":2,"data":...,"expressions":[...]}`.
//! `data` is ordinary JSON, with a null placeholder at every expression leaf.
//! Each entry is exactly `{"path":"/pointer","syntax":"auto","source":"..."}`.
//! Syntax is required: `auto`, `expression` (raw), or `template` (always string).
//! Envelopes and entries must be objects, never positional arrays.
//! Paths are strict RFC6901 pointers: the empty pointer names the root, `/`
//! names an empty object key, and only canonical decimal indices address lists.
//! Data strings and objects never acquire expression meaning by their contents.
//!
//! All envelope and entry fields are required; unknown or duplicate fields,
//! duplicate data properties, unsupported versions, overlapping or duplicate
//! expression paths, missing locations, and non-null placeholders are rejected.
//! Expression-table order does not affect decoding. Encoding follows tree order.
//! Logical depth is bounded at 64, including empty containers; the one outer
//! envelope does not double data nesting or disable the JSON parser's limits.
//! Exact float round-trips require serde_json's `float_roundtrip` feature.
//!
//! All phases serialize through this format, but only authored values deserialize.
//! Sources are retained without compilation, evaluation, or proof construction.
//! Secret-bearing trees fail serialization before writing any envelope content.
//! Borrowing deserializers such as `serde_json::Deserializer` let this module
//! reject unescaped strings and keys before making schema-owned clones. Inputs
//! that require unescaping, and generic deserializers that yield owned strings,
//! may allocate before invoking a visitor; callers must therefore retain an
//! outer request/body byte limit.

use std::{
    borrow::Cow, collections::HashSet, convert::Infallible, fmt, marker::PhantomData, rc::Rc,
};

use indexmap::{IndexMap, map::Entry};
use nebula_expression::{CompiledProgram, ProgramSyntax};
use nebula_validator::foundation::FieldPath as ValuePath;
use serde::{
    Deserialize, Serialize,
    de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor},
    ser::{SerializeMap, SerializeSeq, SerializeStruct},
};
use serde_json::{Number, Value};

use super::{
    MAX_EXPRESSION_ENTRIES, MAX_VALUE_DEPTH,
    budget::ValueBudget,
    tree::{AuthoredValue, ScalarValue, ValueTree},
};
use crate::{Expression, ValidationError};

const AUTHORED_WIRE_VERSION: u16 = 2;

#[derive(Serialize)]
struct ExpressionSlot<'a> {
    path: ValuePath,
    syntax: &'static str,
    source: &'a str,
}

struct ExpressionEntry {
    path: ValuePath,
    syntax: ProgramSyntax,
    source: String,
}

struct ExpressionEntries(Vec<ExpressionEntry>);

struct BorrowedText<'de>(Cow<'de, str>);

impl<'de> Deserialize<'de> for BorrowedText<'de> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BorrowedTextVisitor;

        impl<'de> Visitor<'de> for BorrowedTextVisitor {
            type Value = BorrowedText<'de>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a UTF-8 string")
            }

            fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<Self::Value, E> {
                Ok(BorrowedText(Cow::Borrowed(value)))
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(BorrowedText(Cow::Owned(value.to_owned())))
            }

            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(BorrowedText(Cow::Owned(value)))
            }
        }

        deserializer.deserialize_str(BorrowedTextVisitor)
    }
}

struct ExpressionEntrySeed<'a> {
    budget: &'a ValueBudget,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BorrowedExpressionFields<'a> {
    #[serde(borrow)]
    path: Cow<'a, str>,
    #[serde(deserialize_with = "deserialize_syntax")]
    syntax: ProgramSyntax,
    #[serde(borrow)]
    source: Cow<'a, str>,
}

impl<'de> DeserializeSeed<'de> for ExpressionEntrySeed<'_> {
    type Value = ExpressionEntry;

    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        let accounting_path = ValuePath::root();
        self.budget
            .charge_expression_entry(&accounting_path)
            .map_err(de::Error::custom)?;
        let BorrowedExpressionFields {
            path,
            syntax,
            source,
        } = deserialize_object(deserializer)?;
        self.budget
            .charge_expression_text(path.len().saturating_add(source.len()), &accounting_path)
            .map_err(de::Error::custom)?;
        let path = ValuePath::from_pointer(path.as_ref()).map_err(de::Error::custom)?;
        Ok(ExpressionEntry {
            path,
            syntax,
            source: source.into_owned(),
        })
    }
}

impl<'de> Deserialize<'de> for ExpressionEntries {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct EntriesVisitor;

        impl<'de> Visitor<'de> for EntriesVisitor {
            type Value = ExpressionEntries;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a bounded authored expression table")
            }

            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let capacity = sequence
                    .size_hint()
                    .unwrap_or(0)
                    .min(MAX_EXPRESSION_ENTRIES);
                let mut entries = Vec::with_capacity(capacity);
                let budget = ValueBudget::default();
                while entries.len() < MAX_EXPRESSION_ENTRIES {
                    let Some(entry) =
                        sequence.next_element_seed(ExpressionEntrySeed { budget: &budget })?
                    else {
                        return Ok(ExpressionEntries(entries));
                    };
                    entries.push(entry);
                }
                if sequence.next_element::<IgnoredAny>()?.is_some() {
                    budget
                        .charge_expression_entry(&ValuePath::root())
                        .map_err(de::Error::custom)?;
                }
                Ok(ExpressionEntries(entries))
            }
        }

        deserializer.deserialize_seq(EntriesVisitor)
    }
}

fn syntax_wire_tag(syntax: ProgramSyntax) -> &'static str {
    match syntax {
        ProgramSyntax::Auto => "auto",
        ProgramSyntax::Expression => "expression",
        ProgramSyntax::Template => "template",
    }
}

fn deserialize_syntax<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<ProgramSyntax, D::Error> {
    struct SyntaxVisitor;

    impl Visitor<'_> for SyntaxVisitor {
        type Value = ProgramSyntax;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an authored program syntax")
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
            match value {
                "auto" => Ok(ProgramSyntax::Auto),
                "expression" => Ok(ProgramSyntax::Expression),
                "template" => Ok(ProgramSyntax::Template),
                _ => Err(E::custom("invalid program syntax")),
            }
        }
    }

    deserializer
        .deserialize_str(SyntaxVisitor)
        .map_err(|_| de::Error::custom("invalid program syntax"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredEnvelope {
    version: u16,
    #[serde(deserialize_with = "deserialize_data")]
    data: AuthoredValue,
    expressions: ExpressionEntries,
}

struct DataView<'a, E>(&'a ValueTree<E>);

impl<E> Serialize for DataView<'_, E> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            ValueTree::Literal(value) => value.as_json().serialize(serializer),
            ValueTree::Object(values) => {
                let mut map = serializer.serialize_map(Some(values.len()))?;
                for (key, value) in values {
                    map.serialize_entry(key, &DataView(value))?;
                }
                map.end()
            },
            ValueTree::List(values) => {
                let mut sequence = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    sequence.serialize_element(&DataView(value))?;
                }
                sequence.end()
            },
            ValueTree::Expression(_) => serializer.serialize_unit(),
            ValueTree::Secret(_) => Err(serde::ser::Error::custom(
                "secret-bearing values cannot be serialized as authored data",
            )),
        }
    }
}

#[tracing::instrument(level = "trace", skip_all, fields(wire_version = AUTHORED_WIRE_VERSION))]
fn serialize_authored<'a, E, S: serde::Serializer>(
    tree: &'a ValueTree<E>,
    serializer: S,
    source: impl Fn(&'a E) -> Option<(ProgramSyntax, &'a str)>,
) -> Result<S::Ok, S::Error> {
    let mut expressions = Vec::new();
    collect_sources(
        tree,
        ValuePath::root(),
        0,
        &source,
        &ValueBudget::default(),
        &mut expressions,
    )
    .map_err(serde::ser::Error::custom)?;

    let mut envelope = serializer.serialize_struct("AuthoredValue", 3)?;
    envelope.serialize_field("version", &AUTHORED_WIRE_VERSION)?;
    envelope.serialize_field("data", &DataView(tree))?;
    envelope.serialize_field("expressions", &expressions)?;
    envelope.end()
}

fn collect_sources<'a, E>(
    tree: &'a ValueTree<E>,
    path: ValuePath,
    depth: u8,
    source: &impl Fn(&'a E) -> Option<(ProgramSyntax, &'a str)>,
    budget: &ValueBudget,
    expressions: &mut Vec<ExpressionSlot<'a>>,
) -> Result<(), ValidationError> {
    super::tree::check_depth(&path, depth)?;
    budget.charge_data_node(&path)?;
    match tree {
        ValueTree::Expression(expression) => {
            let (syntax, source) = source(expression).ok_or_else(|| {
                invalid_wire(
                    &path,
                    "expressions cannot be serialized in this value phase",
                )
            })?;
            budget.charge_expression(&path, source)?;
            expressions.push(ExpressionSlot {
                path,
                syntax: syntax_wire_tag(syntax),
                source,
            });
        },
        ValueTree::Object(values) => {
            for (key, value) in values {
                let child_path = path.push(key);
                budget.charge_data_text(key.len(), &child_path)?;
                collect_sources(value, child_path, depth + 1, source, budget, expressions)?;
            }
        },
        ValueTree::List(values) => {
            for (index, value) in values.iter().enumerate() {
                collect_sources(
                    value,
                    path.push(index.to_string()),
                    depth + 1,
                    source,
                    budget,
                    expressions,
                )?;
            }
        },
        ValueTree::Secret(_) => {
            return Err(invalid_wire(
                &path,
                "secret-bearing values cannot be serialized as authored data",
            ));
        },
        ValueTree::Literal(value) => {
            if let Value::String(value) = value.as_json() {
                budget.charge_data_text(value.len(), &path)?;
            }
        },
    }
    Ok(())
}

macro_rules! serialize_tree {
    ($expression:ty, $source:expr) => {
        impl Serialize for ValueTree<$expression> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serialize_authored(self, serializer, $source)
            }
        }
    };
}

serialize_tree!(Expression, |expression: &Expression| Some((
    expression.syntax(),
    expression.source()
)));
serialize_tree!(CompiledProgram, |program: &CompiledProgram| Some((
    program.syntax(),
    program.source()
)));
serialize_tree!(Infallible, |_: &Infallible| None);

impl<'de> Deserialize<'de> for AuthoredValue {
    #[tracing::instrument(level = "trace", skip_all, fields(wire_version = AUTHORED_WIRE_VERSION))]
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut envelope: AuthoredEnvelope = deserialize_object(deserializer)?;
        if envelope.version != AUTHORED_WIRE_VERSION {
            return Err(de::Error::custom("unsupported authored value wire version"));
        }
        validate_expression_paths(&envelope.expressions.0).map_err(de::Error::custom)?;
        for expression in envelope.expressions.0 {
            install_expression(&mut envelope.data, expression).map_err(de::Error::custom)?;
        }
        Ok(envelope.data)
    }
}

fn validate_expression_paths(expressions: &[ExpressionEntry]) -> Result<(), ValidationError> {
    let mut paths = HashSet::with_capacity(expressions.len());
    for expression in expressions {
        if expression
            .path
            .segments()
            .take(usize::from(MAX_VALUE_DEPTH) + 1)
            .count()
            > usize::from(MAX_VALUE_DEPTH)
        {
            return Err(invalid_wire(
                &expression.path,
                "expression path exceeds the value depth limit",
            ));
        }
        if !paths.insert(&expression.path) {
            return Err(invalid_wire(&expression.path, "duplicate expression path"));
        }
    }
    for expression in expressions {
        let mut parent = expression.path.parent();
        while let Some(path) = parent {
            if paths.contains(&path) {
                return Err(invalid_wire(
                    &expression.path,
                    "overlapping expression paths",
                ));
            }
            parent = path.parent();
        }
    }
    Ok(())
}

fn install_expression(
    tree: &mut AuthoredValue,
    expression: ExpressionEntry,
) -> Result<(), ValidationError> {
    let malformed = || {
        invalid_wire(
            &expression.path,
            "expression path must identify a null placeholder",
        )
    };
    let mut current = tree;
    for segment in expression.path.segments() {
        current = match current {
            ValueTree::Object(values) => values.get_mut(segment.as_ref()),
            ValueTree::List(values) => {
                if segment.is_empty()
                    || !segment.bytes().all(|byte| byte.is_ascii_digit())
                    || (segment.starts_with('0') && segment.len() > 1)
                {
                    return Err(malformed());
                }
                let index = segment.parse::<usize>().map_err(|_| malformed())?;
                values.get_mut(index)
            },
            _ => None,
        }
        .ok_or_else(malformed)?;
    }
    if !current.as_literal().is_some_and(Value::is_null) {
        return Err(malformed());
    }
    *current = AuthoredValue::Expression(Expression::with_syntax(
        expression.source,
        expression.syntax,
    ));
    Ok(())
}

fn invalid_wire(path: &ValuePath, message: &'static str) -> ValidationError {
    ValidationError::builder("type_mismatch")
        .at(path.clone())
        .message(message)
        .build()
}

fn deserialize_object<'de, T: Deserialize<'de>, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<T, D::Error> {
    struct ObjectVisitor<T>(PhantomData<T>);

    impl<'de, T: Deserialize<'de>> Visitor<'de> for ObjectVisitor<T> {
        type Value = T;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an authored wire object")
        }

        fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
            T::deserialize(de::value::MapAccessDeserializer::new(map))
        }
    }

    deserializer.deserialize_map(ObjectVisitor(PhantomData))
}

fn deserialize_data<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<AuthoredValue, D::Error> {
    DataSeed {
        path: ValuePath::root(),
        depth: 0,
        budget: Rc::new(ValueBudget::default()),
    }
    .deserialize(deserializer)
}

struct DataSeed {
    path: ValuePath,
    depth: u8,
    budget: Rc<ValueBudget>,
}

impl<'de> DeserializeSeed<'de> for DataSeed {
    type Value = AuthoredValue;

    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        super::tree::check_depth(&self.path, self.depth).map_err(de::Error::custom)?;
        self.budget
            .charge_data_node(&self.path)
            .map_err(de::Error::custom)?;
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for DataSeed {
    type Value = AuthoredValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON data within the value depth limit")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        decode_scalar(Value::Null)
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
        decode_scalar(Value::Bool(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        decode_scalar(Value::Number(value.into()))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        decode_scalar(Value::Number(value.into()))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
        let number = Number::from_f64(value)
            .ok_or_else(|| E::custom("authored data numbers must be finite"))?;
        decode_scalar(Value::Number(number))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        self.budget
            .charge_data_text(value.len(), &self.path)
            .map_err(E::custom)?;
        decode_scalar(Value::String(value.to_owned()))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
        self.budget
            .charge_data_text(value.len(), &self.path)
            .map_err(E::custom)?;
        decode_scalar(Value::String(value))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(DataSeed {
            path: self.path.push(values.len().to_string()),
            depth: self.depth + 1,
            budget: Rc::clone(&self.budget),
        })? {
            values.push(value);
        }
        Ok(AuthoredValue::List(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut values = IndexMap::new();
        while let Some(key) = map.next_key::<BorrowedText<'de>>()? {
            self.budget
                .charge_data_text(key.0.len(), &self.path)
                .map_err(de::Error::custom)?;
            let child_path = self.path.push(key.0.as_ref());
            let key = key.0.into_owned();
            let Entry::Vacant(entry) = values.entry(key) else {
                return Err(de::Error::custom("duplicate data property"));
            };
            let value = map.next_value_seed(DataSeed {
                path: child_path,
                depth: self.depth + 1,
                budget: Rc::clone(&self.budget),
            })?;
            entry.insert(value);
        }
        Ok(AuthoredValue::Object(values))
    }
}

fn decode_scalar<E: de::Error>(value: Value) -> Result<AuthoredValue, E> {
    ScalarValue::try_from(value)
        .map(AuthoredValue::Literal)
        .map_err(E::custom)
}
