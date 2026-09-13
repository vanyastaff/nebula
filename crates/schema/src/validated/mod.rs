//! Validated schema handles — proof-tokens.

use std::{collections::HashSet, sync::Arc};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::{
    error::{ValidationError, ValidationReport},
    field::{Field, ModeField, ModeVariant},
    key::FieldKey,
    loader::{LoaderContext, LoaderRegistry, LoaderResult},
    option::SelectOption,
    path::FieldPath,
    schema::{
        resolve_dynamic_loader_key, resolve_dynamic_loader_path, resolve_select_loader_key,
        resolve_select_loader_path,
    },
    value::{AuthoredValue, ValuePath, ValueTree},
};

mod preparation;
mod root;
mod scalar;
mod typed;
mod validation;
mod values;

pub use root::{RecordShape, RootShape, UnionShape};
pub use scalar::{ScalarKind, ScalarSchema};
pub use validation::PendingValidation;
pub use values::{ResolvedLookup, ResolvedValues, ValidValues};

const MODE_SELECTOR_KEY: &str = "mode";
const MODE_PAYLOAD_KEY: &str = "value";

/// Flags computed once at build time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchemaFlags {
    /// Whether any field allows or requires expressions.
    pub uses_expressions: bool,
    /// Whether any loader-backed field is present (async path).
    pub has_async_loaders: bool,
    /// Maximum nesting depth reached in this schema.
    pub max_depth: u8,
}

/// Cursor into the field tree: breadcrumb of child indices starting from root.
#[derive(Debug, Clone)]
pub struct FieldHandle {
    /// Index path from root fields vec downward.
    pub cursor: SmallVec<[u16; 4]>,
    /// Depth (1 = top-level).
    pub depth: u8,
}

/// The discriminant derived from a schema's authoritative [`RootShape`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SchemaKind {
    /// A concrete record of typed fields, including an empty braced struct.
    #[default]
    Record,
    /// Gradual-typing `Any` — the shape is unknown (`serde_json::Value`).
    Any,
    /// A concrete scalar, including the null wire shape of unit types.
    Scalar,
    /// A tagged union with one required root mode and its serde wire tagging.
    Union,
}

/// How a Rust enum's wire form maps onto a [`SchemaKind::Union`]'s variant keys.
///
/// Recorded on a union schema so the JSON-Schema export can reproduce serde's
/// exact wire shape — preserving the C1 invariant (schema variant key == serde
/// wire key). Only the two taggings with a stable, statically-known discriminant
/// are representable; internally-tagged and untagged enums are rejected at the
/// derive (they cannot satisfy C1: the former inlines the tag into the payload
/// namespace, the latter has no discriminant key at all).
///
/// # Scope
///
/// `serde_tagging` is the single source of truth for a union's wire shape across
/// all three consumers: the `json_schema` export (feature `schemars`), the
/// assignability lattice, and the value-layer bridge —
/// [`ValidSchema::values_from_wire`](ValidSchema::values_from_wire) folds serde's
/// external/adjacent wire (`{"Variant": payload}` / the bare `"Variant"` /
/// `{tag, content}`) into the internal `{mode, value}` envelope that
/// [`validate`](ValidSchema::validate) consumes, and
/// the value projection reconstructs that wire for the typed round-trip.
/// So a derived union guarantees C1 for contract export,
/// static type-checking, *and* runtime value validation of a serialized enum.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SerdeTagging {
    /// serde's default: a data variant is `{"Variant": payload}` and a unit
    /// variant is the bare string `"Variant"`.
    External,
    /// `#[serde(tag = "...", content = "...")]`: a data variant is
    /// `{"<tag>": "Variant", "<content>": payload}` and a unit variant omits the
    /// content key (`{"<tag>": "Variant"}`).
    Adjacent {
        /// The discriminant key (serde `tag`).
        tag: String,
        /// The payload key (serde `content`).
        content: String,
    },
}

/// Shared interior of a `ValidSchema`.
///
#[derive(Debug)]
pub(crate) struct ValidSchemaInner {
    pub root: RootShape,
    /// Flat index from `FieldPath` → `FieldHandle` for O(1) path lookup.
    pub index: IndexMap<FieldPath, FieldHandle>,
    /// Flags computed during build.
    pub flags: SchemaFlags,
    /// Whether validation needs a projected predicate context.
    pub has_contextual_rules: bool,
}

/// Proof-token: schema has been built and linted successfully.
///
/// Cheap to clone — backed by `Arc`.
///
/// Serde: a [`SchemaKind::Record`] serializes as `{"fields": [...]}` (plus
/// `"root_rules": [...]` when [`ValidSchema::root_rules`] is non-empty) —
/// no `kind` tag, so the wire shape is identical to before `kind` existed and a
/// payload with a missing `kind` deserializes back as a record. A
/// [`SchemaKind::Any`] serializes as `{"kind": "any", "fields": []}`.
/// Deserialization rebuilds a record through [`SchemaBuilder`](crate::schema::SchemaBuilder)
/// (invalid wire data returns a [`serde::de::Error`]; lint failures are not
/// panics), and **fails closed** if a payload tagged `kind: "any"` carries any
/// `fields`/`root_rules` instead of silently dropping those constraints.
#[derive(Debug, Clone)]
pub struct ValidSchema(pub(crate) Arc<ValidSchemaInner>);

pub(crate) fn rules_use_predicate_context(rules: &[nebula_validator::Rule]) -> bool {
    let mut references = Vec::new();
    for rule in rules {
        rule.field_references(&mut references);
        if !references.is_empty() {
            return true;
        }
    }
    false
}

impl PartialEq for ValidSchema {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.root == other.0.root
    }
}

impl Eq for ValidSchema {}

impl Serialize for ValidSchema {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        if let Some(scalar) = self.scalar_schema() {
            let mut wire = serializer.serialize_struct("ValidSchema", 2)?;
            wire.serialize_field("kind", &SchemaKind::Scalar)?;
            wire.serialize_field("scalar", scalar)?;
            return wire.end();
        }
        // Preserve historical Record/Any/Union field order and omissions exactly.
        let emit_kind = self.kind() != SchemaKind::Record;
        let emit_tagging = self.serde_tagging().is_some();
        let has_rules = !self.root_rules().is_empty();
        let len = 1 + usize::from(emit_kind) + usize::from(emit_tagging) + usize::from(has_rules);
        let mut s = serializer.serialize_struct("ValidSchema", len)?;
        if emit_kind {
            s.serialize_field("kind", &self.kind())?;
        }
        if let Some(tagging) = self.serde_tagging() {
            s.serialize_field("serde_tagging", tagging)?;
        }
        s.serialize_field("fields", self.fields())?;
        if has_rules {
            s.serialize_field("root_rules", self.root_rules())?;
        }
        s.end()
    }
}

impl<'de> Deserialize<'de> for ValidSchema {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// Transparent wrapper that mirrors `Schema`'s serde representation.
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct ValidSchemaRepr {
            /// Defaults to [`SchemaKind::Record`], so wire data that pre-dates
            /// `kind` (`{"fields": [...]}`) round-trips as a record.
            #[serde(default)]
            kind: SchemaKind,
            /// `Some` only for a [`SchemaKind::Union`]; `None` otherwise.
            #[serde(default, deserialize_with = "deserialize_present")]
            serde_tagging: Option<SerdeTagging>,
            #[serde(default, deserialize_with = "deserialize_present")]
            fields: Option<Vec<Field>>,
            #[serde(default, deserialize_with = "deserialize_present")]
            root_rules: Option<Vec<nebula_validator::Rule>>,
            #[serde(default, deserialize_with = "deserialize_present")]
            scalar: Option<ScalarSchema>,
        }
        let repr = ValidSchemaRepr::deserialize(deserializer)?;
        if repr.kind == SchemaKind::Scalar {
            if repr.fields.is_some() || repr.root_rules.is_some() || repr.serde_tagging.is_some() {
                return Err(serde::de::Error::custom(
                    "scalar roots may carry only their scalar descriptor",
                ));
            }
            let scalar = repr.scalar.ok_or_else(|| {
                serde::de::Error::custom("scalar root requires a scalar descriptor")
            })?;
            return Self::scalar(scalar).map_err(serde::de::Error::custom);
        }
        if repr.scalar.is_some() {
            return Err(serde::de::Error::custom(
                "only scalar roots may carry a scalar descriptor",
            ));
        }
        let fields = repr.fields.unwrap_or_default();
        let root_rules = repr.root_rules.unwrap_or_default();
        if repr.kind == SchemaKind::Union {
            // Fail closed: a union is exactly one required root `Field::Mode`
            // plus a `serde_tagging`. Reconstruct through `ValidSchema::union`
            // so the same lint / index / required-root invariants the in-process
            // constructor enforces also gate wire-loaded unions — a malformed
            // shape (wrong field count, non-`Mode` root, missing tagging, stray
            // root rules) is rejected, never silently coerced into a permissive
            // schema.
            let Some(tagging) = repr.serde_tagging else {
                return Err(serde::de::Error::custom(
                    "schema tagged `kind: \"union\"` must carry `serde_tagging`",
                ));
            };
            if !root_rules.is_empty() {
                return Err(serde::de::Error::custom(
                    "schema tagged `kind: \"union\"` must not carry `root_rules`",
                ));
            }
            if fields.len() != 1 {
                return Err(serde::de::Error::custom(
                    "union schema must carry exactly one root mode field",
                ));
            }
            let Some(Field::Mode(mode)) = fields.into_iter().next() else {
                return Err(serde::de::Error::custom(
                    "union schema's root field must be a mode field",
                ));
            };
            return Self::union(mode, tagging)
                .map_err(|report| serde::de::Error::custom(format!("invalid union: {report:?}")));
        }
        // `serde_tagging` belongs only to a union; a `Record`/`Any` carrying it is
        // malformed — reject rather than silently drop it.
        if repr.serde_tagging.is_some() {
            return Err(serde::de::Error::custom(
                "only a schema tagged `kind: \"union\"` may carry `serde_tagging`",
            ));
        }
        if repr.kind == SchemaKind::Any {
            // Fail closed: an `Any` schema carries no constraints. A payload
            // tagged `Any` that also lists `fields`/`root_rules` is malformed
            // (mistagged, or from a mixed-version producer); silently returning
            // the unconstrained `any()` would drop every constraint and make
            // validation fully permissive. Reject it instead.
            if !fields.is_empty() || !root_rules.is_empty() {
                return Err(serde::de::Error::custom(
                    "schema tagged `kind: \"any\"` must not carry `fields` or `root_rules`",
                ));
            }
            return Ok(Self::any());
        }
        let mut b = fields.into_iter().fold(
            crate::schema::SchemaBuilder::default(),
            super::schema::SchemaBuilder::add,
        );
        for rule in root_rules {
            b = b.root_rule(rule);
        }
        b.build()
            .map_err(|report| serde::de::Error::custom(format!("invalid schema: {report:?}")))
    }
}

fn deserialize_present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

impl ValidSchema {
    pub(crate) fn from_inner(inner: ValidSchemaInner) -> Self {
        Self(Arc::new(inner))
    }

    /// Shared empty `ValidSchema` — cheap `Arc` clone.
    ///
    /// This describes an empty braced record, not the null wire shape of `()`.
    pub fn empty() -> Self {
        use std::sync::OnceLock;
        static EMPTY: OnceLock<ValidSchema> = OnceLock::new();
        EMPTY
            .get_or_init(|| {
                Self::from_inner(ValidSchemaInner {
                    root: RootShape::record(Vec::new(), Vec::new()),
                    index: IndexMap::new(),
                    flags: SchemaFlags::default(),
                    has_contextual_rules: false,
                })
            })
            .clone()
    }

    /// Shared gradual-typing `Any` `ValidSchema` — cheap `Arc` clone.
    ///
    /// Use this for inputs whose shape is unknown at the type level
    /// (`serde_json::Value`, [`AuthoredValue`]). It is field-less like [`empty`](Self::empty),
    /// but its [`kind`](Self::kind) is [`SchemaKind::Any`], so the assignability
    /// lattice treats it as the gradual `Any` rather than an empty record —
    /// keeping the two from collapsing into one another in the type-DAG.
    pub fn any() -> Self {
        use std::sync::OnceLock;
        static ANY: OnceLock<ValidSchema> = OnceLock::new();
        ANY.get_or_init(|| {
            Self::from_inner(ValidSchemaInner {
                root: RootShape::Any,
                index: IndexMap::new(),
                flags: SchemaFlags::default(),
                has_contextual_rules: false,
            })
        })
        .clone()
    }

    /// Build a scalar root with checked rules and no expression admission.
    ///
    /// # Errors
    /// Returns root-rule lint failures, including references to absent fields.
    #[tracing::instrument(name = "schema.scalar.build", skip_all, fields(kind = ?scalar.kind()))]
    pub fn scalar(scalar: ScalarSchema) -> Result<Self, ValidationReport> {
        let mut report = ValidationReport::new();
        crate::lint::lint_root_rules(scalar.root_rules(), &[], &mut report);
        if report.has_errors() {
            tracing::debug!(
                error_count = report.errors().count(),
                "scalar schema lint rejected"
            );
            return Err(report);
        }
        let has_contextual_rules = rules_use_predicate_context(scalar.root_rules());
        Ok(Self::from_inner(ValidSchemaInner {
            root: RootShape::Scalar(scalar),
            index: IndexMap::new(),
            flags: SchemaFlags::default(),
            has_contextual_rules,
        }))
    }

    /// The authoritative root contract; kind and field views are derived from it.
    #[must_use]
    pub fn root_shape(&self) -> &RootShape {
        &self.0.root
    }

    /// The scalar contract, if this schema has a scalar root.
    #[must_use]
    pub fn scalar_schema(&self) -> Option<&ScalarSchema> {
        match self.root_shape() {
            RootShape::Scalar(scalar) => Some(scalar),
            _ => None,
        }
    }

    /// Build a tagged-union (sum-type) schema from its variants.
    ///
    /// `mode_field` carries the union's variants — one
    /// [`ModeVariant`] per Rust enum variant, its key
    /// the serde wire discriminant; `tagging` records how the enum's wire form
    /// maps onto those keys (see [`SerdeTagging`]). The union is stored as the
    /// schema's sole **required** root [`Field::Mode`] (see [`SchemaKind::Union`]),
    /// so it is built through the normal
    /// [`SchemaBuilder`](crate::schema::SchemaBuilder) — running the same field
    /// lint (variant-key validity and uniqueness), index build, and depth/fan-out
    /// guard as any other schema — then retagged as a union. The root mode is
    /// forced [`RequiredMode::Always`](crate::RequiredMode): a sum-type value is
    /// never optional, so an absent value must fail rather than validate clean.
    ///
    /// # Errors
    ///
    /// Returns the [`ValidationReport`] from the builder when a variant key is not
    /// a valid [`FieldKey`], two variants collide, or the schema otherwise fails a
    /// build-time lint.
    pub fn union(mode_field: ModeField, tagging: SerdeTagging) -> Result<Self, ValidationReport> {
        // A tagged union has no default variant: serde always requires the
        // discriminant on the wire, and mode validation otherwise falls back to a
        // `default_variant` when the selector is absent — which would let a value
        // with no discriminant validate, breaking the tagged-union contract.
        if mode_field.default_variant.is_some() {
            return Err(ValidationReport::from(
                ValidationError::builder("union.default_variant")
                    .message(
                        "a tagged union has no default variant — serde always requires the \
                         discriminant; remove `default_variant` before building the union",
                    )
                    .build(),
            ));
        }
        // Build through the normal builder so the union runs the same field lint
        // (variant-key validity / uniqueness), index, and depth guard; `build_union`
        // stamps the kind + tagging during construction (no post-build `Arc`
        // surgery, no panic path).
        crate::schema::Schema::builder()
            .add(mode_field.required())
            .build_union(tagging)
    }

    /// Ingest an external **serde wire** value into the `{mode, value}` envelope
    /// that [`validate`](Self::validate) consumes.
    ///
    /// For a [`Record`](SchemaKind::Record) or [`Any`](SchemaKind::Any) schema this
    /// is exactly [`AuthoredValue::from_data`]: the serde wire already matches the
    /// validator's key-space. For a [`Union`](SchemaKind::Union) it rewrites serde's
    /// external/adjacent tagging — `{"Variant": payload}` / the bare string
    /// `"Variant"` (external) and `{<tag>: "Variant", <content>: payload}` /
    /// `{<tag>: "Variant"}` (adjacent) — into the internal envelope
    /// `{<root>: {"mode": "Variant", "value": payload}}` keyed under the union's sole
    /// root [`Field::Mode`], driven by the stored [`SerdeTagging`]. This closes the
    /// C1 value-layer gap so a derived union validates the serde wire its
    /// `#[derive(Serialize)]` actually emits. All strings and objects remain data,
    /// including template-like strings and objects with a `$expr` property.
    ///
    /// Unit variants carry no `value` (they validate against the hidden
    /// [`ModeField::EMPTY_PLACEHOLDER_KEY`](crate::field::ModeField::EMPTY_PLACEHOLDER_KEY)
    /// payload); data variants carry their payload under `value`.
    ///
    /// # Errors
    ///
    /// Returns `union.unknown_variant` when the wire discriminant is not a declared
    /// variant, `union.malformed_wire` when the wire shape does not match the
    /// declared tagging (a non-string/non-object external value, an external object
    /// without exactly one key, an adjacent value missing or mistyping the tag, a
    /// stray adjacent key, or a data shape for a unit variant and vice-versa), or any
    /// `recursion_limit` for over-deep nesting. Arbitrary JSON property names are
    /// preserved; schema validation subsequently checks the required shape.
    #[tracing::instrument(
        level = "debug",
        target = "nebula_schema::union_ingress",
        skip(self, wire),
        fields(kind = ?self.kind())
    )]
    pub fn values_from_wire(
        &self,
        wire: serde_json::Value,
    ) -> Result<AuthoredValue, ValidationError> {
        match self.kind() {
            SchemaKind::Record | SchemaKind::Any | SchemaKind::Scalar => {
                AuthoredValue::from_data(wire)
            },
            SchemaKind::Union => AuthoredValue::from_data(self.rewrite_union_wire(wire)?),
        }
    }

    /// Rewrite a serde-tagged union wire value into the `{<root>: {mode, value?}}`
    /// envelope. Driven entirely by the stored [`SerdeTagging`] + the root mode's
    /// declared variants, so the schema's `serde_tagging` is the single source of
    /// truth for the wire shape (no per-consumer divergence).
    fn rewrite_union_wire(
        &self,
        wire: serde_json::Value,
    ) -> Result<serde_json::Value, ValidationError> {
        use serde_json::Value;

        let malformed = |message: String| {
            ValidationError::builder("union.malformed_wire")
                .message(message)
                .build()
        };

        let Some(root_field) = self.fields().first() else {
            return Err(malformed(
                "union schema is missing its root mode field".to_owned(),
            ));
        };
        let Field::Mode(mode) = root_field else {
            return Err(malformed(
                "union schema's root field is not a mode field".to_owned(),
            ));
        };
        let Some(tagging) = self.serde_tagging() else {
            return Err(malformed(
                "union schema is missing its serde tagging".to_owned(),
            ));
        };

        // Pull the wire discriminant + optional payload out of serde's tagging.
        let (variant_key, payload): (String, Option<Value>) = match tagging {
            SerdeTagging::External => match wire {
                // serde external unit variant: the bare string `"Variant"`.
                Value::String(variant) => (variant, None),
                // serde external data variant: `{"Variant": payload}` — exactly one key.
                Value::Object(map) => {
                    let mut entries = map.into_iter();
                    let (Some((key, payload)), None) = (entries.next(), entries.next()) else {
                        return Err(malformed(
                            "external union data variant must be a single-key object `{\"Variant\": payload}`"
                                .to_owned(),
                        ));
                    };
                    (key, Some(payload))
                },
                _ => {
                    return Err(malformed(
                        "external union wire must be a string (unit variant) or a single-key \
                         object (data variant)"
                            .to_owned(),
                    ));
                },
            },
            SerdeTagging::Adjacent { tag, content } => {
                let Value::Object(mut map) = wire else {
                    return Err(malformed(format!(
                        "adjacent union wire must be an object carrying the tag `{tag}`"
                    )));
                };
                let Some(Value::String(variant)) = map.remove(tag.as_str()) else {
                    return Err(malformed(format!(
                        "adjacent union wire must carry a string discriminant under the tag `{tag}`"
                    )));
                };
                let payload = map.remove(content.as_str());
                if !map.is_empty() {
                    return Err(malformed(format!(
                        "adjacent union wire may only carry the tag `{tag}` and content `{content}` keys"
                    )));
                }
                (variant, payload)
            },
        };

        // The discriminant must be one of the union's declared variants.
        let Some(variant) = mode.variants.iter().find(|v| v.key == variant_key) else {
            return Err(ValidationError::builder("union.unknown_variant")
                .param("variant", Value::String(variant_key.clone()))
                .message(format!(
                    "`{variant_key}` is not a declared variant of this union"
                ))
                .build());
        };

        // A unit variant carries no payload (the hidden EMPTY_PLACEHOLDER_KEY field);
        // a data variant must. Cross-check the wire shape against the schema so a
        // mismatch is a precise error rather than a confusing payload type error.
        let is_unit = variant.field.key().as_str() == ModeField::EMPTY_PLACEHOLDER_KEY;
        match (is_unit, &payload) {
            (true, Some(_)) => {
                return Err(malformed(format!(
                    "unit variant `{variant_key}` carries no payload"
                )));
            },
            (false, None) => {
                return Err(malformed(format!(
                    "data variant `{variant_key}` requires a payload"
                )));
            },
            _ => {},
        }

        // Build `{<root>: {"mode": "Variant", "value"?: payload}}`.
        let mut envelope = serde_json::Map::with_capacity(2);
        envelope.insert(MODE_SELECTOR_KEY.to_owned(), Value::String(variant_key));
        if let Some(payload) = payload {
            envelope.insert(MODE_PAYLOAD_KEY.to_owned(), payload);
        }
        let mut out = serde_json::Map::with_capacity(1);
        out.insert(
            root_field.key().as_str().to_owned(),
            Value::Object(envelope),
        );
        Ok(Value::Object(out))
    }

    /// Reconstruct a union's external serde wire from its internal data view.
    ///
    /// This consumes JSON data, not authored tagged serialization. Non-union
    /// values pass through unchanged. A malformed union envelope also passes
    /// through unchanged; only validation can confer schema validity.
    pub(crate) fn raw_values_to_wire(&self, mut data: serde_json::Value) -> serde_json::Value {
        use serde_json::Value;

        if self.kind() != SchemaKind::Union {
            return data;
        }
        let (Some(Field::Mode(mode)), Some(tagging)) =
            (self.fields().first(), self.serde_tagging())
        else {
            return data;
        };
        let Some(envelope) = data
            .get_mut(mode.key.as_str())
            .and_then(Value::as_object_mut)
        else {
            return data;
        };
        let Some(variant_key) = envelope
            .get(MODE_SELECTOR_KEY)
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return data;
        };
        let Some(variant) = mode
            .variants
            .iter()
            .find(|variant| variant.key == variant_key)
        else {
            return data;
        };
        let is_unit = variant.field.key().as_str() == ModeField::EMPTY_PLACEHOLDER_KEY;
        if envelope
            .keys()
            .any(|key| key != MODE_SELECTOR_KEY && key != MODE_PAYLOAD_KEY)
            || is_unit == envelope.contains_key(MODE_PAYLOAD_KEY)
        {
            return data;
        }

        let payload = envelope.remove(MODE_PAYLOAD_KEY);
        match tagging {
            SerdeTagging::External if is_unit => Value::String(variant_key),
            SerdeTagging::External => {
                let mut wire = serde_json::Map::with_capacity(1);
                wire.insert(variant_key, payload.unwrap_or(Value::Null));
                Value::Object(wire)
            },
            SerdeTagging::Adjacent { tag, content } => {
                let mut wire = serde_json::Map::with_capacity(2);
                wire.insert(tag.clone(), Value::String(variant_key));
                if let Some(payload) = payload {
                    wire.insert(content.clone(), payload);
                }
                Value::Object(wire)
            },
        }
    }

    /// The path of the first value key that no field of this schema declares —
    /// the closed-set query, walked through nested objects, list
    /// items, and a union's active variant payload (`mode`/`value` are structural
    /// keys of a [`Field::Mode`] envelope, not flagged). `None` when every key in
    /// `values` maps onto a declared field. Paths use RFC6901, preserving arbitrary
    /// property names and empty segments. Read aliases follow preparation's
    /// canonical precedence; opaque field contents are not declared-key scopes.
    ///
    /// [`validate`](Self::validate) deliberately *ignores* undeclared keys (an
    /// open record tolerates extra wire fields); this is the complementary
    /// closed-set check a consumer applies when undeclared input must be rejected
    /// rather than silently dropped — e.g. `nebula-resource` rejecting an inlined
    /// secret-shaped field in a `ResourceConfig` (which must carry no secrets) at
    /// any depth, not just the top level. For the gradual-typing
    /// [`Any`](SchemaKind::Any) schema **every** key is "undeclared", so callers
    /// that treat gradual typing as unchecked must gate on [`Self::kind`] being
    /// `Any`. An empty [`Record`](SchemaKind::Record) is concrete and declares
    /// that every key is undeclared.
    #[must_use]
    pub fn first_undeclared_path<E>(&self, values: &ValueTree<E>) -> Option<ValuePath> {
        first_undeclared_in_level(self.fields(), values.as_object()?, &ValuePath::root())
    }

    /// Whether this schema is a concrete [`Record`](SchemaKind::Record), the
    /// gradual-typing [`Any`](SchemaKind::Any), or a tagged
    /// [`Union`](SchemaKind::Union).
    #[must_use]
    pub fn kind(&self) -> SchemaKind {
        self.0.root.kind()
    }

    /// The serde tagging of a [`Union`](SchemaKind::Union) schema, or `None` for a
    /// `Record`/`Any`.
    #[must_use]
    pub fn serde_tagging(&self) -> Option<&SerdeTagging> {
        self.0.root.serde_tagging()
    }

    /// Return `true` when two `ValidSchema` values share the same backing
    /// `Arc` — i.e. they're the same instance, not just structurally
    /// equivalent. Used to assert identity-preserving caches (e.g. the
    /// `OnceLock` inside `#[derive(Schema)]`).
    #[must_use]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Borrow all top-level fields in insertion order.
    #[must_use]
    pub fn fields(&self) -> &[Field] {
        self.0.root.fields()
    }

    /// Borrow the build-time flags.
    #[must_use]
    pub fn flags(&self) -> &SchemaFlags {
        &self.0.flags
    }

    /// Schema-level rules run after per-field validation (see [`ValidSchema::validate`]).
    #[must_use]
    pub fn root_rules(&self) -> &[nebula_validator::Rule] {
        self.0.root.root_rules()
    }

    pub(super) fn has_contextual_rules(&self) -> bool {
        self.0.has_contextual_rules
    }

    /// Find a top-level field by key.
    #[must_use]
    pub fn find(&self, key: &FieldKey) -> Option<&Field> {
        self.fields().iter().find(|f| f.key() == key)
    }

    /// Find a field by dotted path using the O(1) index.
    #[must_use]
    pub fn find_by_path(&self, path: &FieldPath) -> Option<&Field> {
        let handle = self.0.index.get(path)?;
        let mut cur = self.fields().get(*handle.cursor.first()? as usize)?;
        for &step in &handle.cursor[1..] {
            cur = match cur {
                Field::Object(o) => o.fields.get(step as usize)?,
                Field::List(l) => l.item.as_deref()?,
                Field::Mode(m) => &m.variants.get(step as usize)?.field,
                _ => return None,
            };
        }
        Some(cur)
    }

    /// Walk a `Reference` parameter's canonical RFC6901 `output_path` through this
    /// schema's field tree, gating on field **opacity** at every step (ADR-0100
    /// TypeDAG, W0 U5).
    ///
    /// Consumes already-decoded [`ValuePath`] segments. This is intentionally
    /// **not** [`Self::find_by_path`]
    /// (which index-jumps straight to a leaf and conflates "absent" with "under
    /// an opaque node") — it re-walks field-by-field so opacity can be checked at
    /// every intermediate node, not just the destination.
    ///
    /// # Design: four outcomes, not two
    ///
    /// A `Reference`'s path is only checkable end-to-end when *every* node from
    /// root to leaf is **closed** (provably, exhaustively typed). Real producer
    /// shapes are routinely partly opaque (a `serde_json::Value` sub-field, a
    /// `Select`/`File` that may emit an array, an adjacent-tagged enum), so a
    /// binary resolve/fail-to-resolve verdict would either hard-reject those
    /// legitimate workflows or silently stop checking anything. [`PathWalk`]
    /// instead separates "provably wrong" ([`PathWalk::Unresolved`]) from "hit
    /// something we cannot reason about" ([`PathWalk::Opaque`], the caller's
    /// fail-open exit) — only the former is ever a hard error.
    ///
    /// ## Field classification (exhaustive — every arm is deliberate)
    ///
    /// - **Closed scalar leaf** — `String`/`Secret`/`Number`/`Boolean`/`Code`. A
    ///   further segment past one of these is provably wrong:
    ///   [`PathResolveError::DescendPastLeaf`].
    /// - **`Select`/`File`/`Notice` — opaque, NOT scalar.** A `multiple` `Select`
    ///   or `File` yields a JSON array, and a `Select` option value or a single
    ///   `File` value is commonly an object; `Notice` produces no runtime value
    ///   at all. None is safe to treat as a childless terminal the way the closed
    ///   scalars are.
    /// - **Non-empty `Object`** — closed *only* for descending into a key that is
    ///   actually declared; a missing key is [`PathWalk::Opaque`], **never** a
    ///   hard error. [`HasSchema`](crate::HasSchema) is a public, **unsealed**
    ///   trait with real hand-written impls in the tree, and [`Field::Object`]
    ///   carries no exhaustiveness marker distinguishing a derive-guaranteed-
    ///   complete object from a possibly-incomplete hand-written one — so a
    ///   non-empty `Object` cannot be trusted as an exhaustive declaration of
    ///   every key the real value may carry. Treating a missing key as "unknown"
    ///   rather than "wrong" is the load-bearing, deliberately conservative
    ///   choice here.
    /// - **Empty `Object`** — opaque (the dominant real shape for a nested
    ///   `serde_json::Value` field, which derives to a fieldless `Object`, not
    ///   `Any`).
    /// - **`List`** — closed *for the node itself* (a numeric-index segment is
    ///   required: [`PathResolveError::NonIndexOnList`] otherwise), but its
    ///   `item` is reclassified at the descent step: `None` (untyped) or an
    ///   opaque item (e.g. `Vec<serde_json::Value>`) → [`PathWalk::Opaque`].
    /// - **`Mode`/`Dynamic`/`Computed`/`Unknown`, or any `Field` variant this
    ///   version does not yet know about** — opaque, via an explicit wildcard
    ///   arm so a future variant defaults to opaque rather than silently
    ///   becoming hard-errorable.
    ///
    /// # Returns
    ///
    /// - [`PathWalk::ResolvedRoot`] — the path is the root pointer and this is a
    ///   concrete `Record` or `Scalar` schema. A caller comparing that complete
    ///   output to one consumer field uses
    ///   [`explain_root_field_assignable`](crate::explain_root_field_assignable).
    /// - [`PathWalk::Opaque`] — the root schema is an `Any`/`Union`/future kind,
    ///   the root segment names no declared field, or the walk hit an opaque
    ///   node / missing key at any later step.
    /// - [`PathWalk::Unresolved`] — a non-numeric segment against a `List`, or a
    ///   segment past a closed scalar leaf, on a path that was otherwise
    ///   fully-closed up to that point.
    /// - [`PathWalk::Resolved`] — every node from root to leaf was closed; the
    ///   leaf field the path resolves to.
    #[must_use]
    pub fn walk_reference_path(&self, reference_path: &ValuePath) -> PathWalk<'_> {
        let mut segments = reference_path.segments();

        let Some(root_key) = segments.next() else {
            return if matches!(self.kind(), SchemaKind::Record | SchemaKind::Scalar) {
                PathWalk::ResolvedRoot
            } else {
                PathWalk::Opaque
            };
        };
        if self.kind() == SchemaKind::Scalar {
            return PathWalk::Unresolved(PathResolveError::DescendPastLeaf {
                segment: root_key.into_owned(),
            });
        }
        if self.kind() != SchemaKind::Record {
            return PathWalk::Opaque;
        }
        let Some(root_field) = self
            .fields()
            .iter()
            .find(|field| field.key().as_str() == root_key.as_ref())
        else {
            return PathWalk::Opaque;
        };

        // Walk each remaining segment, gating opacity at every step.
        let mut current = root_field;
        for segment in segments {
            match walk_step(current, segment.as_ref()) {
                PathStep::Advance(next) => current = next,
                PathStep::Opaque => return PathWalk::Opaque,
                PathStep::Unresolved(err) => return PathWalk::Unresolved(err),
            }
        }

        // Classify the final leaf the same way as every intermediate node.
        if is_opaque_field_node(current) {
            PathWalk::Opaque
        } else {
            PathWalk::Resolved(current)
        }
    }

    /// Wrap a single `Field` under a synthetic `key` into a one-field schema, so
    /// [`explain_assignable`](crate::explain_assignable) — which pairs a
    /// producer/consumer field by [`Field::key`] equality — can be reused at
    /// **field granularity**. Crate-private: the only caller is
    /// [`explain_field_assignable`](crate::compat::explain_field_assignable),
    /// which builds these, compares them, and discards them within the same
    /// function call — this bypassed-pipeline schema is never handed back to
    /// an outside caller (see that function's docs for why that matters).
    ///
    /// `field` is **re-keyed** to `key`, not merely wrapped under it: a
    /// producer's resolved path leaf carries its own intrinsic key (the last
    /// path segment — e.g. `"email"` for `/contact/email`), which need not equal
    /// the consumer parameter key the caller is checking it against. Re-keying
    /// both the producer leaf and the consumer field to the same synthetic key
    /// is what lets the assignability check's key-based field pairing match them
    /// up. (`Field::Unknown`'s key is private to this module and is left
    /// un-rekeyed; harmless because every caller gates on the field being
    /// non-opaque first, and `Field::Unknown` is always opaque — see
    /// [`Self::walk_reference_path`].)
    ///
    /// Bypasses the full lint / index-build pipeline `SchemaBuilder::build` runs
    /// (mirrors [`Self::empty`]/[`Self::any`]): `field` was already lint-clean
    /// inside its source schema, and the assignability check reads only
    /// [`Self::fields`]/[`Self::kind`] — never the path index or build-time
    /// flags — so an empty index and default flags are safe for this narrow,
    /// internal, single-comparison use.
    pub(crate) fn single_field(key: FieldKey, field: Field) -> ValidSchema {
        Self::from_inner(ValidSchemaInner {
            root: RootShape::record(vec![rekeyed(field, key)], Vec::new()),
            index: IndexMap::new(),
            flags: SchemaFlags::default(),
            has_contextual_rules: false,
        })
    }

    /// Resolve dynamic options for a select field through loader registry.
    ///
    /// Same error taxonomy as [`crate::Schema::load_select_options`], but this
    /// entrypoint guarantees validated schema invariants.
    ///
    /// # Errors
    ///
    /// Returns `invalid_key`, `loader.not_registered`, or `loader.failed`.
    pub async fn load_select_options(
        &self,
        key: &str,
        registry: &LoaderRegistry,
        context: LoaderContext,
    ) -> Result<LoaderResult<SelectOption>, ValidationError> {
        let loader_key = resolve_select_loader_key(self.fields(), key)?;
        registry
            .load_options(&loader_key, context.redacted(self.fields())?)
            .await
    }

    /// Resolve dynamic options for a select field at a nested schema path.
    ///
    /// Uses the same schema-path addressing rules as
    /// [`crate::Schema::load_select_options_at`].
    ///
    /// # Errors
    ///
    /// Returns `field.not_found`, `field.type_mismatch`, `loader.missing_config`,
    /// `loader.not_registered`, or `loader.failed`.
    pub async fn load_select_options_at(
        &self,
        path: &FieldPath,
        registry: &LoaderRegistry,
        context: LoaderContext,
    ) -> Result<LoaderResult<SelectOption>, ValidationError> {
        let loader_key = resolve_select_loader_path(self.fields(), path)?;
        registry
            .load_options(&loader_key, context.redacted(self.fields())?)
            .await
    }

    /// Resolve dynamic record payloads for a dynamic field through registry.
    ///
    /// Same error taxonomy as [`crate::Schema::load_dynamic_records`], but this
    /// entrypoint guarantees validated schema invariants.
    ///
    /// # Errors
    ///
    /// Returns `invalid_key`, `loader.not_registered`, or `loader.failed`.
    pub async fn load_dynamic_records(
        &self,
        key: &str,
        registry: &LoaderRegistry,
        context: LoaderContext,
    ) -> Result<LoaderResult<serde_json::Value>, ValidationError> {
        let loader_key = resolve_dynamic_loader_key(self.fields(), key)?;
        registry
            .load_records(&loader_key, context.redacted(self.fields())?)
            .await
    }

    /// Resolve dynamic record payloads for a field at a nested schema path.
    ///
    /// Uses the same schema-path addressing rules as
    /// [`crate::Schema::load_dynamic_records_at`].
    ///
    /// # Errors
    ///
    /// Returns `field.not_found`, `field.type_mismatch`, `loader.missing_config`,
    /// `loader.not_registered`, or `loader.failed`.
    pub async fn load_dynamic_records_at(
        &self,
        path: &FieldPath,
        registry: &LoaderRegistry,
        context: LoaderContext,
    ) -> Result<LoaderResult<serde_json::Value>, ValidationError> {
        let loader_key = resolve_dynamic_loader_path(self.fields(), path)?;
        registry
            .load_records(&loader_key, context.redacted(self.fields())?)
            .await
    }

    /// Consume authored values through preparation and staged validation.
    ///
    /// Preparation admits and compiles expressions, canonicalizes aliases,
    /// applies literal transformers once, and protects secrets before any
    /// validation context or proof token is constructed. Deferred obligations
    /// are retained for resolution through the compiled programs.
    ///
    /// # Errors
    ///
    /// Returns a report if preparation or input-phase validation fails.
    ///
    /// # Example
    ///
    /// ```rust
    /// use nebula_schema::{AuthoredValue, Field, Schema, field_key};
    /// use serde_json::json;
    ///
    /// let schema = Schema::builder()
    ///     .add(Field::string(field_key!("name")).required())
    ///     .build()
    ///     .unwrap();
    /// assert!(schema.validate(AuthoredValue::from_data(json!({})).unwrap()).is_err());
    /// assert!(schema.validate(AuthoredValue::from_data(json!({"name": "Alice"})).unwrap()).is_ok());
    /// ```
    #[tracing::instrument(
        level = "debug",
        target = "nebula_schema::validate",
        skip(self, values),
        fields(field_count = self.fields().len(), has_root_rules = !self.root_rules().is_empty())
    )]
    pub fn validate(&self, values: AuthoredValue) -> Result<ValidValues, ValidationReport> {
        values::validate_input(self, values)
    }

    /// Project authored input into redacted data with output-key remaps.
    ///
    /// Read aliases follow the same precedence as preparation. Declared secrets
    /// are omitted recursively; extra record keys remain data, but cannot
    /// occupy canonical or remapped output names. Expressions remain explicit
    /// source envelopes, never evaluated by projection. Union output uses the
    /// schema's serde tagging rather than the internal validation envelope.
    ///
    /// # Errors
    ///
    /// Returns `recursion_limit` for over-deep input, or `type_mismatch` for a
    /// non-object input to a record or union schema.
    #[tracing::instrument(level = "trace", skip_all, fields(field_count = self.fields().len()))]
    pub fn project(&self, values: &AuthoredValue) -> Result<serde_json::Value, ValidationError> {
        values.check_depth(&ValuePath::root(), 0)?;
        if matches!(self.kind(), SchemaKind::Record | SchemaKind::Union)
            && values.as_object().is_none()
        {
            return Err(ValidationError::builder("type_mismatch")
                .message("record and union projections require an object")
                .build());
        }
        if let Some(scalar) = self.scalar_schema() {
            scalar.validate_value(values, &ValuePath::root())?;
        }
        Ok(self.raw_values_to_wire(project_tree(
            self.fields(),
            values,
            &|expression| serde_json::json!({"$expr": expression.source()}),
        )))
    }
}

// ── Reference-path opacity walk (ADR-0100 TypeDAG, W0 U5) ───────────────────

/// The result of [`ValidSchema::walk_reference_path`] — see that method for the
/// full field-opacity classification.
///
/// A plain `Option`/`Result` cannot distinguish a resolved whole-schema root,
/// a resolved field, an opaque target, and a provably invalid descent.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum PathWalk<'a> {
    /// The root pointer resolves to the complete concrete `Record` or `Scalar`
    /// schema supplied to the walk. The schema remains available to the caller
    /// as the receiver of [`ValidSchema::walk_reference_path`].
    ResolvedRoot,
    /// Every node from root to leaf was closed (fully, provably typed); here is
    /// the resolved leaf field.
    Resolved(&'a Field),
    /// A provable structural mistake on an otherwise fully-closed path.
    Unresolved(PathResolveError),
    /// The walk hit an opaque node, a missing `Object` key, or an untyped/opaque
    /// `List` item before it could prove or refute anything — the caller must
    /// treat the reference as unchecked, not invalid.
    Opaque,
}

/// Why a reference path is provably wrong on an otherwise fully-closed walk
/// (see [`ValidSchema::walk_reference_path`]). Never returned for a missing
/// `Object` key or any other opaque node — those are
/// [`PathWalk::Opaque`], not this.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathResolveError {
    /// A `List` step's segment does not parse as a `usize` index.
    #[error("path segment `{segment}` is not a valid list index")]
    NonIndexOnList {
        /// The offending non-numeric segment.
        segment: String,
    },
    /// A further segment appears after a closed scalar leaf
    /// (`String`/`Secret`/`Number`/`Boolean`/`Code`), which has no children to
    /// descend into.
    #[error("path segment `{segment}` appears after a scalar leaf, which has no children")]
    DescendPastLeaf {
        /// The segment with nowhere to go.
        segment: String,
    },
}

/// Outcome of classifying one further path `segment` against `current` in
/// [`ValidSchema::walk_reference_path`].
enum PathStep<'a> {
    /// Advance the walk into this child field.
    Advance(&'a Field),
    /// Opacity (or a missing key) — the caller fails open.
    Opaque,
    /// A provable structural mistake.
    Unresolved(PathResolveError),
}

/// Whether `field` is opaque **as a landing node** — unknowable without a
/// runtime value in hand. This is the exhaustive classification
/// [`ValidSchema::walk_reference_path`] documents in full; the wildcard arm here
/// is deliberate (not `todo!()`/`unreachable!()`): any `Field` variant this
/// version does not yet recognize defaults to opaque rather than silently
/// becoming hard-errorable when a new variant is added.
///
/// - Closed scalar (`String`/`Secret`/`Number`/`Boolean`/`Code`): not opaque.
/// - `List`: not opaque as the node itself — its `item` is reclassified at the
///   descent step in `walk_step`, never here.
/// - Non-empty `Object`: not opaque (closed for descending into a key that is
///   actually declared).
/// - Everything else — empty `Object`, `Select`, `File`, `Notice`, `Mode`,
///   `Dynamic`, `Computed`, `Unknown`, any future variant — is opaque.
///
/// Public because a caller that already has a single resolved [`Field`] in
/// hand (not a whole schema) needs the SAME classification to decide whether
/// that field is determinable on its own — e.g. `nebula-workflow`'s
/// `check_reference_edges` applies this to a consumer's declared parameter
/// field to decide whether the per-field type check
/// ([`explain_field_assignable`](crate::compat::explain_field_assignable)) can
/// run at all, or must fail open. Exposing this `&Field -> bool` predicate
/// (rather than routing through a synthetic schema) keeps that reuse honest:
/// no impostor [`ValidSchema`] ever needs to leave this crate for it.
#[must_use]
pub fn is_opaque_field_node(field: &Field) -> bool {
    match field {
        Field::Object(obj) => obj.fields.is_empty(),
        Field::String(_)
        | Field::Secret(_)
        | Field::Number(_)
        | Field::Boolean(_)
        | Field::Code(_)
        | Field::List(_) => false,
        _ => true,
    }
}

/// Classify one further path `segment` against `current`, dispatching on
/// `current`'s kind. See [`ValidSchema::walk_reference_path`] for the full
/// rationale of each arm.
fn walk_step<'a>(current: &'a Field, segment: &str) -> PathStep<'a> {
    match current {
        Field::Object(obj) if !obj.fields.is_empty() => {
            match obj.fields.iter().find(|f| f.key().as_str() == segment) {
                Some(next) => PathStep::Advance(next),
                // A missing key fails open rather than hard-erroring: `HasSchema`
                // is unsealed, so a non-empty `Object` cannot be trusted as an
                // exhaustive declaration of every key the real value may carry.
                None => PathStep::Opaque,
            }
        },
        Field::List(list) => {
            if segment.parse::<usize>().is_err() {
                return PathStep::Unresolved(PathResolveError::NonIndexOnList {
                    segment: segment.to_owned(),
                });
            }
            match list.item.as_deref() {
                Some(item) if !is_opaque_field_node(item) => PathStep::Advance(item),
                // `None` (untyped item) or an opaque item type (e.g. a
                // `Vec<serde_json::Value>`, which derives to an opaque item field).
                _ => PathStep::Opaque,
            }
        },
        Field::String(_)
        | Field::Secret(_)
        | Field::Number(_)
        | Field::Boolean(_)
        | Field::Code(_) => PathStep::Unresolved(PathResolveError::DescendPastLeaf {
            segment: segment.to_owned(),
        }),
        // Empty `Object`, `Select`, `File`, `Notice`, `Mode`, `Dynamic`,
        // `Computed`, `Unknown`, or any future `Field` variant: opaque.
        _ => PathStep::Opaque,
    }
}

/// Re-key `field` to `key`, preserving everything else about it. See
/// `ValidSchema::single_field` for why the caller needs this rather than a
/// plain wrap.
fn rekeyed(mut field: Field, key: FieldKey) -> Field {
    match &mut field {
        Field::String(f) => f.key = key,
        Field::Secret(f) => f.key = key,
        Field::Number(f) => f.key = key,
        Field::Boolean(f) => f.key = key,
        Field::Select(f) => f.key = key,
        Field::Object(f) => f.key = key,
        Field::List(f) => f.key = key,
        Field::Mode(f) => f.key = key,
        Field::Code(f) => f.key = key,
        Field::File(f) => f.key = key,
        Field::Computed(f) => f.key = key,
        Field::Dynamic(f) => f.key = key,
        Field::Notice(f) => f.key = key,
        // `UnknownField.key` is private to this crate's `field` module, so it
        // cannot be rewritten from here — left as its original key. Never
        // reached in practice: every caller of `single_field` gates on the
        // field being non-opaque first, and `Field::Unknown` is always opaque
        // (see `is_opaque_field_node`).
        Field::Unknown(_) => {},
    }
    field
}

/// Project a depth-checked tree. Public raw-input callers check depth first;
/// proof-token callers already retain a bounded, prepared tree.
pub(super) fn project_tree<E>(
    fields: &[Field],
    values: &ValueTree<E>,
    expression: &impl Fn(&E) -> serde_json::Value,
) -> serde_json::Value {
    match values {
        ValueTree::Object(properties) => project_level(fields, properties, expression),
        _ => values.json_with(expression),
    }
}

/// Borrow a scope through preparation's alias policy without cloning subtrees.
fn canonical_property_refs<'v, E>(
    fields: &[Field],
    properties: &'v IndexMap<String, ValueTree<E>>,
) -> IndexMap<String, &'v ValueTree<E>> {
    let mut canonical = properties
        .iter()
        .map(|(key, value)| (key.clone(), value))
        .collect();
    preparation::fold_aliases(fields, &mut canonical);
    canonical
}

fn project_level<E>(
    fields: &[Field],
    properties: &IndexMap<String, ValueTree<E>>,
    expression: &impl Fn(&E) -> serde_json::Value,
) -> serde_json::Value {
    use serde_json::Value;

    let canonical = canonical_property_refs(fields, properties);
    let mut out = serde_json::Map::new();
    // Reserve schema-owned outputs even when the field is absent or secret.
    let reserved: HashSet<&str> = fields
        .iter()
        .flat_map(|field| {
            std::iter::once(field.key().as_str()).chain(field.emit_as().map(FieldKey::as_str))
        })
        .collect();

    for field in fields {
        let Some(value) = canonical.get(field.key().as_str()) else {
            continue;
        };
        let Some(projected) = project_value(field, value, expression) else {
            continue;
        };
        let output_key = field.emit_as().unwrap_or_else(|| field.key()).as_str();
        out.insert(output_key.to_owned(), projected);
    }
    for (key, value) in canonical {
        if !reserved.contains(key.as_str()) {
            out.insert(key, value.json_with(expression));
        }
    }
    Value::Object(out)
}

/// Drop declared secrets and malformed secret-bearing containers at every scope.
fn project_value<E>(
    field: &Field,
    value: &ValueTree<E>,
    expression: &impl Fn(&E) -> serde_json::Value,
) -> Option<serde_json::Value> {
    use serde_json::Value;

    if matches!(field, Field::Secret(_)) || matches!(value, ValueTree::Secret(_)) {
        return None;
    }
    match (field, value) {
        (Field::Object(object), ValueTree::Object(properties)) => {
            Some(project_level(&object.fields, properties, expression))
        },
        (Field::List(list), ValueTree::List(items)) => match list.item.as_deref() {
            Some(item_field) => Some(Value::Array(
                items
                    .iter()
                    .filter_map(|item| project_value(item_field, item, expression))
                    .collect(),
            )),
            None => Some(value.json_with(expression)),
        },
        (Field::Mode(mode), ValueTree::Object(properties)) => {
            Some(project_mode_object(mode, properties, expression))
        },
        _ if crate::context::field_subtree_has_secret(field) => None,
        _ => Some(value.json_with(expression)),
    }
}

/// An explicit string selector wins; an absent selector permits the declared
/// default. An invalid or unknown explicit selector never selects a default.
fn active_mode_variant_for_object<'m, E>(
    mode: &'m ModeField,
    properties: &IndexMap<String, ValueTree<E>>,
) -> Option<&'m ModeVariant> {
    let key = match properties.get(MODE_SELECTOR_KEY) {
        Some(value) => value.as_str()?,
        None => mode.default_variant.as_deref()?,
    };
    mode.variants.iter().find(|variant| variant.key == key)
}

fn project_mode_object<E>(
    mode: &ModeField,
    properties: &IndexMap<String, ValueTree<E>>,
    expression: &impl Fn(&E) -> serde_json::Value,
) -> serde_json::Value {
    use serde_json::Value;

    let mut out = serde_json::Map::new();
    if let Some(selector) = properties
        .get(MODE_SELECTOR_KEY)
        .and_then(ValueTree::as_str)
    {
        out.insert(
            MODE_SELECTOR_KEY.to_owned(),
            Value::String(selector.to_owned()),
        );
    }
    if let Some(payload) = properties.get(MODE_PAYLOAD_KEY)
        && let Some(variant) = active_mode_variant_for_object(mode, properties)
        && let Some(projected) = project_value(&variant.field, payload, expression)
    {
        out.insert(MODE_PAYLOAD_KEY.to_owned(), projected);
    }
    Value::Object(out)
}

enum UndeclaredEntry<'a, E> {
    Level {
        fields: &'a [Field],
        properties: &'a IndexMap<String, ValueTree<E>>,
        path: ValuePath,
    },
    Field {
        field: &'a Field,
        value: &'a ValueTree<E>,
        path: ValuePath,
    },
    Unknown(ValuePath),
}

/// Iterative depth-first traversal avoids both a recursive stack overflow and
/// treating a depth cutoff as evidence that all keys were declared.
fn first_undeclared_in_level<E>(
    fields: &[Field],
    properties: &IndexMap<String, ValueTree<E>>,
    path: &ValuePath,
) -> Option<ValuePath> {
    let mut pending = vec![UndeclaredEntry::Level {
        fields,
        properties,
        path: path.clone(),
    }];
    while let Some(entry) = pending.pop() {
        match entry {
            UndeclaredEntry::Unknown(path) => return Some(path),
            UndeclaredEntry::Level {
                fields,
                properties,
                path,
            } => {
                for (key, value) in canonical_property_refs(fields, properties)
                    .into_iter()
                    .rev()
                {
                    let child_path = path.push(&key);
                    pending.push(
                        match fields.iter().find(|field| field.key().as_str() == key) {
                            Some(field) => UndeclaredEntry::Field {
                                field,
                                value,
                                path: child_path,
                            },
                            None => UndeclaredEntry::Unknown(child_path),
                        },
                    );
                }
            },
            UndeclaredEntry::Field { field, value, path } => match (field, value) {
                (Field::Object(object), ValueTree::Object(properties)) => {
                    pending.push(UndeclaredEntry::Level {
                        fields: &object.fields,
                        properties,
                        path,
                    });
                },
                (Field::List(list), ValueTree::List(items)) => {
                    if let Some(field) = list.item.as_deref() {
                        for (index, value) in items.iter().enumerate().rev() {
                            pending.push(UndeclaredEntry::Field {
                                field,
                                value,
                                path: path.push(index.to_string()),
                            });
                        }
                    }
                },
                (Field::Mode(mode), ValueTree::Object(properties)) => {
                    let variant = active_mode_variant_for_object(mode, properties);
                    for (key, value) in properties.iter().rev() {
                        if key == MODE_PAYLOAD_KEY {
                            if let Some(variant) = variant {
                                pending.push(UndeclaredEntry::Field {
                                    field: &variant.field,
                                    value,
                                    path: path.push(key),
                                });
                            }
                        } else if key != MODE_SELECTOR_KEY {
                            pending.push(UndeclaredEntry::Unknown(path.push(key)));
                        }
                    }
                },
                _ => {},
            },
        }
    }
    None
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod value_helper_tests {
    use serde_json::{Value, json};

    use super::{SerdeTagging, ValidSchema};
    use crate::{AuthoredValue, Expression, Field, ResolvedValue, Schema, ValueTree, field_key};

    fn assert_data_only(value: &AuthoredValue) {
        let mut pending = vec![value];
        while let Some(value) = pending.pop() {
            match value {
                ValueTree::Literal(_) => {},
                ValueTree::Object(properties) => pending.extend(properties.values()),
                ValueTree::List(items) => pending.extend(items),
                ValueTree::Expression(_) | ValueTree::Secret(_) => {
                    panic!("wire data must not acquire expression or secret syntax")
                },
            }
        }
    }

    fn union(tagging: SerdeTagging) -> ValidSchema {
        ValidSchema::union(
            Field::mode(field_key!("auth"))
                .variant("data", "Data", Field::object(field_key!("payload")))
                .variant_empty("none", "None"),
            tagging,
        )
        .unwrap()
    }

    #[test]
    fn wire_ingress_preserves_arbitrary_data_without_authored_interpretation() {
        let data = json!({
            "": "{{ $data.source }}",
            "a/b~": {"$expr": "$data.source"},
            "\u{e9}": [{"kind": "expression", "value": "$data.source"}],
        });
        for schema in [ValidSchema::empty(), ValidSchema::any()] {
            let values = schema.values_from_wire(data.clone()).unwrap();
            assert_data_only(&values);
            assert_eq!(values.to_json(), data);
            assert_eq!(schema.raw_values_to_wire(values.to_json()), data);
        }
        for data in [
            Value::Null,
            json!(7),
            json!("{{ $data.x }}"),
            json!([1, {"": 2}]),
        ] {
            let values = ValidSchema::any().values_from_wire(data.clone()).unwrap();
            assert_data_only(&values);
            assert_eq!(values.to_json(), data);
        }
    }

    #[test]
    fn union_data_and_unit_wire_roundtrip_for_both_taggings() {
        let payload = json!({"": "{{ $data.x }}", "$expr": "$data.y", "a/b~": [1, 2]});
        for (schema, data_wire, unit_wire) in [
            (
                union(SerdeTagging::External),
                json!({"data": payload}),
                json!("none"),
            ),
            (
                union(SerdeTagging::Adjacent {
                    tag: "type".into(),
                    content: "body".into(),
                }),
                json!({"type": "data", "body": payload}),
                json!({"type": "none"}),
            ),
        ] {
            for wire in [data_wire, unit_wire] {
                let values = schema.values_from_wire(wire.clone()).unwrap();
                assert_data_only(&values);
                assert_eq!(schema.raw_values_to_wire(values.to_json()), wire);
            }
        }
    }

    #[test]
    fn malformed_union_reverse_mapping_does_not_discard_data() {
        let schema = union(SerdeTagging::External);
        for data in [
            json!({"auth": {"mode": "missing", "value": {"": 1}}}),
            json!({"auth": {"mode": "none", "value": null}}),
            json!({"auth": {"mode": "data"}}),
            json!({"auth": {"mode": "data", "value": {}, "extra": true}}),
            json!({"auth": {"mode": {"unexpected": 1}}}),
        ] {
            assert_eq!(schema.raw_values_to_wire(data.clone()), data);
        }
    }

    #[test]
    fn undeclared_paths_preserve_empty_and_escaped_keys_across_stages() {
        let schema = Schema::builder()
            .add(Field::object(field_key!("nested")))
            .build()
            .unwrap();
        for key in ["", "/", "~", "a/b~", "\u{e9}"] {
            let data = json!({"nested": {key: 1}});
            let expected = crate::ValuePath::root().push("nested").push(key);
            let authored = AuthoredValue::from_data(data.clone()).unwrap();
            let resolved = ResolvedValue::from_data(data).unwrap();
            assert_eq!(
                schema.first_undeclared_path(&authored),
                Some(expected.clone())
            );
            assert_eq!(schema.first_undeclared_path(&resolved), Some(expected));
        }
        let empty_key = AuthoredValue::from_data(json!({"": null})).unwrap();
        assert_eq!(
            ValidSchema::empty()
                .first_undeclared_path(&empty_key)
                .unwrap()
                .as_str(),
            "/"
        );
    }

    #[test]
    fn undeclared_paths_walk_list_payloads_and_reject_extra_mode_keys() {
        let schema = Schema::builder()
            .add(Field::list(field_key!("rows")).item(Field::object(field_key!("row"))))
            .build()
            .unwrap();
        let values = AuthoredValue::from_data(json!({"rows": [{"a/b~": true}]})).unwrap();
        assert_eq!(
            schema.first_undeclared_path(&values).unwrap().as_str(),
            "/rows/0/a~1b~0"
        );

        let schema = union(SerdeTagging::External);
        let values = schema
            .values_from_wire(json!({"data": {"": true}}))
            .unwrap();
        assert_eq!(
            schema.first_undeclared_path(&values).unwrap().as_str(),
            "/auth/value/"
        );
        let values =
            AuthoredValue::from_data(json!({"auth": {"mode": "none", "extra": 1}})).unwrap();
        assert_eq!(
            schema.first_undeclared_path(&values).unwrap().as_str(),
            "/auth/extra"
        );
    }

    #[test]
    fn projection_folds_aliases_and_omits_secrets_without_mutating_input() {
        let schema = Schema::builder()
            .add(
                Field::object(field_key!("settings"))
                    .read_alias("legacy_settings")
                    .unwrap()
                    .add(
                        Field::string(field_key!("id"))
                            .read_alias("legacy_id")
                            .unwrap()
                            .emit_as("public_id")
                            .unwrap(),
                    )
                    .add(
                        Field::secret(field_key!("token"))
                            .read_alias("legacy_token")
                            .unwrap(),
                    ),
            )
            .build()
            .unwrap();
        let values = AuthoredValue::from_data(json!({"legacy_settings": {
            "id": "canonical", "legacy_id": "losing",
            "token": "protected", "legacy_token": "also protected",
            "public_id": "spoofed", "": true, "a/b~": 3
        }}))
        .unwrap();
        let before = values.clone();
        assert_eq!(
            schema.project(&values).unwrap(),
            json!({"settings": {
                "public_id": "canonical", "": true, "a/b~": 3
            }})
        );
        assert_eq!(values, before);
    }

    #[test]
    fn projection_reserves_absent_outputs_and_preserves_expression_envelopes() {
        let schema = Schema::builder()
            .add(
                Field::string(field_key!("id"))
                    .emit_as("public_id")
                    .unwrap(),
            )
            .build()
            .unwrap();
        let values = AuthoredValue::from_data(json!({"public_id": "spoofed", "a/b~": 1})).unwrap();
        assert_eq!(schema.project(&values).unwrap(), json!({"a/b~": 1}));

        let mut values = AuthoredValue::object();
        values
            .insert("id", AuthoredValue::Expression(Expression::new("$data.id")))
            .unwrap();
        assert_eq!(
            schema.project(&values).unwrap(),
            json!({"public_id": {"$expr": "$data.id"}})
        );
    }

    #[test]
    fn malformed_mode_selectors_do_not_leak_payload_or_select_defaults() {
        let schema = Schema::builder()
            .add(
                Field::mode(field_key!("auth"))
                    .variant(
                        "known",
                        "Known",
                        Field::object(field_key!("payload"))
                            .add(Field::string(field_key!("id")))
                            .add(Field::secret(field_key!("token"))),
                    )
                    .default_variant("known"),
            )
            .build()
            .unwrap();
        let values = AuthoredValue::from_data(json!({"auth": {
            "mode": {"token": "must not escape"}, "value": {"id": "not selected", "token": "protected"}
        }})).unwrap();
        assert_eq!(schema.project(&values).unwrap(), json!({"auth": {}}));
        let values = AuthoredValue::from_data(
            json!({"auth": {"value": {"id": "selected", "token": "protected"}}}),
        )
        .unwrap();
        assert_eq!(
            schema.project(&values).unwrap(),
            json!({"auth": {"value": {"id": "selected"}}})
        );
    }

    #[test]
    fn public_projection_checks_raw_depth_before_the_recursive_walk() {
        let schema = ValidSchema::any();
        let mut data = Value::Null;
        for _ in 0..crate::value::MAX_VALUE_DEPTH {
            data = json!([data]);
        }
        let values = AuthoredValue::from_data(data.clone()).unwrap();
        assert_eq!(schema.project(&values).unwrap(), data);
        let over_depth = AuthoredValue::List(vec![values]);
        assert_eq!(
            schema.project(&over_depth).unwrap_err().code(),
            "recursion_limit"
        );
        let scalar = AuthoredValue::from_data(json!(1)).unwrap();
        assert_eq!(
            ValidSchema::empty().project(&scalar).unwrap_err().code(),
            "type_mismatch"
        );
    }
}
