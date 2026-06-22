//! Schema-compatibility check: structural width-subtyping (TypeDAG T1).
//!
//! The kernel of the ADR-0100 connection type-check. Called by the workflow
//! per-edge validator (T3) to decide whether a producer node's `Output` schema
//! is assignable where a consumer node's `Input` schema is expected.
//!
//! The public entry point is [`is_assignable_schema`], which takes two
//! [`ValidSchema`] values so it can honor the [`SchemaKind`] Top/Bottom split:
//! `is_assignable_schema(&producer.output, &consumer.input)`. (An internal,
//! kind-blind slice form is used only by this module's tests.)

use crate::{Field, FieldKey, RequiredMode, SchemaKind, ValidSchema};

// ── Public types ─────────────────────────────────────────────────────────────

/// Why a producer schema is not assignable to a consumer schema.
///
/// Returned by [`is_assignable_schema`] when the structural width-subtyping
/// check fails. Carries the first incompatibility found (depth-first,
/// consumer-field order).
///
/// This enum is `#[non_exhaustive]` — new incompatibility kinds (e.g. semantic
/// type constraints) may be added in future minor versions without breaking
/// existing `match` arms.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SchemaIncompat {
    /// A consumer field with [`RequiredMode::Always`] has no counterpart in the
    /// producer schema.
    #[error("missing required field `{key}`")]
    MissingRequiredField {
        /// Key of the missing required field.
        key: FieldKey,
    },
    /// A field present on both sides has incompatible types (different `Field`
    /// variants). The `producer` and `consumer` strings are the
    /// [`Field::type_name`] values — `"string"`, `"number"`, etc.
    #[error(
        "field `{key}` type mismatch: producer has `{producer}`, consumer expects `{consumer}`"
    )]
    FieldTypeMismatch {
        /// Key of the mismatched field.
        key: FieldKey,
        /// Type name reported by the producer field.
        producer: &'static str,
        /// Type name reported by the consumer field.
        consumer: &'static str,
    },
    /// A field present on both sides has the same structural variant (e.g. both
    /// are `Object` or both are `List`), but the nested fields are themselves
    /// incompatible. The `key` is the outer field; `inner` carries the first
    /// incompatibility found inside.
    ///
    /// This allows callers to distinguish "the outer field has the right type
    /// but a nested field is wrong" from "the outer field is a completely
    /// different type".
    #[error("nested incompatibility in field `{key}`")]
    NestedIncompat {
        /// Key of the outer (container) field.
        key: FieldKey,
        /// The first incompatibility found inside the container.
        #[source]
        inner: Box<SchemaIncompat>,
    },
    /// A `File` or `Select` field is present on both sides but the `multiple`
    /// cardinality differs (scalar vs. array), making the wire shapes incompatible.
    #[error(
        "field `{key}` cardinality mismatch: \
         producer multiple={producer_multiple}, consumer expects multiple={consumer_multiple}"
    )]
    CardinalityMismatch {
        /// Key of the mismatched field.
        key: FieldKey,
        /// Whether the producer field allows multiple values.
        producer_multiple: bool,
        /// Whether the consumer field expects multiple values.
        consumer_multiple: bool,
    },
}

// ── Public entry point ───────────────────────────────────────────────────────

/// Kind-aware structural width-subtyping: are producer values assignable where
/// consumer values are expected? (`Output <: Input`, Liskov.)
///
/// This is **the** public assignability check (ADR-0100 T1). The workflow
/// per-edge validator (T3) and the action output-evolution check both call it,
/// so the [`SchemaKind`] Top/Bottom split is enforced on real producer→consumer
/// edges, not merely at the type/serde level.
///
/// ```rust
/// use nebula_schema::{Field, Schema, ValidSchema, field_key, is_assignable_schema};
///
/// let producer = Schema::builder()
///     .add(Field::string(field_key!("name")).required())
///     .add(Field::number(field_key!("extra")))
///     .build()
///     .unwrap();
/// let consumer = Schema::builder()
///     .add(Field::string(field_key!("name")).required())
///     .build()
///     .unwrap();
/// // Width subtyping: the producer has every required consumer field (+ extras).
/// assert!(is_assignable_schema(&producer, &consumer).is_ok());
///
/// // An empty *record* producer (`()`) provably emits nothing, so it does NOT
/// // satisfy a consumer that hard-requires a field …
/// assert!(is_assignable_schema(&ValidSchema::empty(), &consumer).is_err());
/// // … whereas the gradual `Any` (`serde_json::Value`) still passes.
/// assert!(is_assignable_schema(&ValidSchema::any(), &consumer).is_ok());
/// ```
///
/// # Kinds (Top/Bottom split)
///
/// - **Gradual `Any` escape** — if either side is [`SchemaKind::Any`] (e.g.
///   `serde_json::Value`), the producer may emit anything and the consumer
///   accepts anything, so the check passes; untyped interop is preserved.
/// - **Strict record subtyping** — when **both** sides are concrete records,
///   width-subtyping is enforced *without* the empty-producer escape: an empty
///   record produces no fields, so it does **not** satisfy a consumer that
///   hard-requires any (it returns [`SchemaIncompat::MissingRequiredField`]). An
///   empty record *consumer* still accepts everything (it requires nothing).
///
/// Strict mode removes only the empty-*record*-producer escape. The per-field
/// gradual escapes below (`Dynamic`/`Computed` fields, and `List` fields with no
/// typed item) still pass in **both** modes by construction. A primitive or
/// `serde_json::Value` output is [`SchemaKind::Any`], so a bare-scalar producer
/// can never be statically rejected against a record consumer — gradual typing
/// at the leaf (wrap the value in a `#[derive(Schema)]` struct for strict typing).
///
/// # Record subtyping rules (ADR-0100 §L1/L2)
///
/// - **Width subtyping** — the consumer's required fields must be a subset of
///   the producer's fields with type-compatible matches on the overlap. Extra
///   producer fields are ignored.
/// - **`Dynamic`/`Computed` fields** are treated as `Any` on either side.
/// - **`Notice` fields** are display-only and ignored on the consumer side.
/// - Only [`RequiredMode::Always`] consumer fields are hard requirements;
///   [`RequiredMode::When`] and the default optional mode are not enforced
///   statically (the runtime condition cannot be proved at validation time).
/// - **`File` and `Select` cardinality** — the `multiple` flag (scalar vs.
///   array) is checked for equality; a mismatch returns
///   [`SchemaIncompat::CardinalityMismatch`].
/// - **`Mode` fields** — `Mode`-vs-`Mode` passes the binary check (never
///   false-rejects), but [`explain_assignable`] reports it as
///   [`UnknownReason::ModeVariance`]: sum-type variance is not modeled.
/// - **`Number` integer vs. float** — both carry `type_name() == "number"`.
///   int→float is provably compatible; float→int passes the binary check but is
///   [`UnknownReason::NumberWidening`] in [`explain_assignable`] (possible
///   precision loss).
///
/// # Errors
///
/// When both sides are concrete records and the strict check fails, returns the
/// first [`SchemaIncompat`] found (depth-first, consumer-field order):
/// - [`SchemaIncompat::MissingRequiredField`] — a hard-required consumer field
///   has no counterpart in the producer.
/// - [`SchemaIncompat::FieldTypeMismatch`] — a field present on both sides
///   carries incompatible types (different `Field` variants).
/// - [`SchemaIncompat::NestedIncompat`] — a field present on both sides has the
///   same structural variant (both `Object` or both `List`) but the nested
///   fields are incompatible; wraps the inner [`SchemaIncompat`].
/// - [`SchemaIncompat::CardinalityMismatch`] — a `File` or `Select` field is
///   present on both sides but the `multiple` flag differs.
#[must_use = "check the Result — an Err means the producer is not assignable to the consumer"]
pub fn is_assignable_schema(
    producer: &ValidSchema,
    consumer: &ValidSchema,
) -> Result<(), SchemaIncompat> {
    // Binary view of the ternary verdict: `Yes` and `Unknown` (not statically
    // refuted) both pass — this keeps the gradual escape (untyped producers,
    // Dynamic/Mode/Number leniencies) green. Only a definite `No` is an error,
    // surfaced as its first incompatibility (depth-first, consumer-field order).
    match explain_assignable(producer, consumer) {
        Assignability::Yes | Assignability::Unknown(_) => Ok(()),
        // `into_verdict` only builds `No` from a non-empty list, so `next()` is
        // always `Some`; `map_or` keeps this total without a panic path.
        Assignability::No(incompats) => incompats.into_iter().next().map_or(Ok(()), Err),
    }
}

/// The three-valued (Cue/GraphQL-style) assignability verdict: a producer
/// schema is provably assignable, provably not, or **not statically decidable**.
///
/// Unlike [`is_assignable_schema`] — which collapses to a binary `Ok`/`Err` and
/// returns only the *first* incompatibility — this verdict separates "not
/// provable" ([`Unknown`](Assignability::Unknown)) from "provably wrong"
/// ([`No`](Assignability::No)) and collects **every** finding, so a strict
/// validator can block on unprovable edges while a gradual one passes them. The
/// `No`/`Unknown` lists are non-empty in their respective variants.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assignability {
    /// The producer is provably assignable to the consumer.
    Yes,
    /// The producer is provably **not** assignable; carries every incompatibility
    /// found (depth-first, consumer-field order), not just the first.
    No(Vec<SchemaIncompat>),
    /// Assignability cannot be decided statically (e.g. a loader-backed
    /// `Dynamic` field, an opaque `Any` producer, sum-type `Mode` variance, or
    /// a float→int narrowing). **Not** fail-open: a strict policy treats this as
    /// a blocked edge; a gradual policy passes it.
    Unknown(Vec<UnknownReason>),
}

/// Why an edge's assignability is [`Unknown`](Assignability::Unknown) — each
/// case is a place where the type system cannot currently *prove* compatibility
/// nor refute it.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnknownReason {
    /// The producer schema is the gradual `Any` ([`SchemaKind::Any`]): it may
    /// emit anything, so it is not provably compatible with a typed consumer.
    OpaqueProducer,
    /// A matched field is `Dynamic`/`Computed` (loader- or expression-backed),
    /// so its concrete shape is unknown until runtime.
    DynamicLoaderBacked {
        /// Key of the dynamic field.
        key: FieldKey,
    },
    /// A matched `Mode` (sum-type) pair: variant-level variance is not modeled,
    /// so neither compatibility nor a conflict is proven.
    ModeVariance {
        /// Key of the `Mode` field.
        key: FieldKey,
    },
    /// A matched `Number` pair narrows float→int: the producer may emit a
    /// non-integral value the consumer cannot represent, but a static check
    /// cannot tell whether it ever will.
    ///
    /// This is deliberately `Unknown`, not a hard `No` (contrast the
    /// empty-record producer, which *provably* emits nothing and so is `No`):
    /// the schema layer has no integral-domain refinement, so a `float` producer
    /// is not *provably* non-integral. A future numeric-refinement type could
    /// promote this to `Yes` or `No`; until then it is genuinely undecidable.
    NumberWidening {
        /// Key of the number field.
        key: FieldKey,
    },
    /// An undecidable reason found inside a nested `Object` or `List` field,
    /// carrying the enclosing field `key` so the path is not lost (mirrors
    /// [`SchemaIncompat::NestedIncompat`]). Nesting composes: an `Object` two
    /// levels deep yields `NestedUnknown { a, NestedUnknown { b, .. } }`.
    NestedUnknown {
        /// Key of the enclosing container field.
        key: FieldKey,
        /// The undecidable reason found inside the container.
        inner: Box<UnknownReason>,
    },
}

impl core::fmt::Display for UnknownReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::OpaqueProducer => {
                write!(f, "producer schema is an opaque `Any` (shape unknown)")
            },
            Self::DynamicLoaderBacked { key } => {
                write!(f, "field `{key}` is dynamic/computed (resolved at runtime)")
            },
            Self::ModeVariance { key } => {
                write!(
                    f,
                    "field `{key}` is a `Mode` sum type (variance not modeled)"
                )
            },
            Self::NumberWidening { key } => {
                write!(
                    f,
                    "field `{key}` narrows float to integer (possible precision loss)"
                )
            },
            Self::NestedUnknown { key, inner } => write!(f, "in field `{key}`: {inner}"),
        }
    }
}

/// Full three-valued assignability: collects **all** incompatibilities and all
/// "not decidable" reasons rather than stopping at the first.
///
/// Honors the [`SchemaKind`] Top/Bottom split like [`is_assignable_schema`]: an
/// `Any` consumer accepts anything ([`Yes`](Assignability::Yes)); an `Any`
/// producer is [`Unknown(OpaqueProducer)`](UnknownReason::OpaqueProducer); two
/// concrete records run strict width-subtyping. The verdict precedence is
/// `No` (any provable incompatibility) ▸ `Unknown` (any undecidable field) ▸
/// `Yes`.
///
/// The leniencies that [`is_assignable_schema`] silently passes are surfaced
/// here as [`Unknown`](Assignability::Unknown) instead of being hidden in `Ok`,
/// so a strict workflow validator can block them. See [`UnknownReason`].
#[must_use]
pub fn explain_assignable(producer: &ValidSchema, consumer: &ValidSchema) -> Assignability {
    // An `Any` consumer accepts anything — provably compatible.
    if consumer.kind() == SchemaKind::Any {
        return Assignability::Yes;
    }
    // An `Any` producer may emit anything: not refuted, but not provable either.
    if producer.kind() == SchemaKind::Any {
        return Assignability::Unknown(vec![UnknownReason::OpaqueProducer]);
    }
    let mut acc = Explain::default();
    collect_fields(producer.fields(), consumer.fields(), true, &mut acc);
    acc.into_verdict()
}

// ── Private core ─────────────────────────────────────────────────────────────

/// Accumulator for the collect-all traversal: every provable incompatibility
/// and every undecidable reason, in depth-first / consumer-field order.
#[derive(Default)]
struct Explain {
    incompat: Vec<SchemaIncompat>,
    unknown: Vec<UnknownReason>,
}

impl Explain {
    /// Collapse to a verdict: `No` if any incompatibility (a provable conflict
    /// dominates), else `Unknown` if any undecidable field, else `Yes`.
    fn into_verdict(self) -> Assignability {
        if !self.incompat.is_empty() {
            Assignability::No(self.incompat)
        } else if !self.unknown.is_empty() {
            Assignability::Unknown(self.unknown)
        } else {
            Assignability::Yes
        }
    }
}

impl Explain {
    /// Fold a nested sub-traversal's findings into this accumulator under `key`,
    /// wrapping each nested incompatibility in [`SchemaIncompat::NestedIncompat`]
    /// and each undecidable reason in [`UnknownReason::NestedUnknown`] — so both
    /// channels keep the enclosing-field path (e.g. `config.token` rather than a
    /// bare `token` that two sibling objects could both produce).
    fn wrap_nested(&mut self, key: &FieldKey, sub: Explain) {
        for inner in sub.incompat {
            self.incompat.push(SchemaIncompat::NestedIncompat {
                key: key.clone(),
                inner: Box::new(inner),
            });
        }
        for inner in sub.unknown {
            self.unknown.push(UnknownReason::NestedUnknown {
                key: key.clone(),
                inner: Box::new(inner),
            });
        }
    }
}

/// Collect-all field-slice traversal, shared by [`explain_assignable`] (which
/// passes `strict = true`) and the test-only `explain_slice` helper (which
/// controls `strict` explicitly). Pushes every incompatibility and every
/// undecidable reason into `acc` in depth-first, consumer-field order; it never
/// early-returns, so the caller always sees the full picture.
fn collect_fields(
    producer_fields: &[Field],
    consumer_fields: &[Field],
    strict: bool,
    acc: &mut Explain,
) {
    // An empty consumer requires nothing.
    if consumer_fields.is_empty() {
        return;
    }
    // Gradual mode: an empty producer is the untyped/opaque `Any` escape. Strict
    // mode (the kind-aware path) gives it no escape — an empty record emits no
    // fields and must face the per-field required check below.
    if !strict && producer_fields.is_empty() {
        return;
    }

    for consumer_field in consumer_fields {
        // Notice fields are display-only (not data flow) — skip entirely.
        if matches!(consumer_field, Field::Notice(_)) {
            continue;
        }

        let is_hard_required = matches!(consumer_field.required(), RequiredMode::Always);
        let consumer_key = consumer_field.key();

        match producer_fields.iter().find(|pf| pf.key() == consumer_key) {
            None if is_hard_required => {
                acc.incompat.push(SchemaIncompat::MissingRequiredField {
                    key: consumer_key.clone(),
                });
            },
            // Optional consumer field absent from producer — fine under width subtyping.
            None => {},
            Some(producer_field) => {
                collect_pair(consumer_key, producer_field, consumer_field, strict, acc);
            },
        }
    }
}

/// Classify a matched field pair (same key, both present), pushing into `acc`.
///
/// Provable conflicts (cardinality, type mismatch, nested) become
/// [`SchemaIncompat`]; the leniencies the binary check silently passes —
/// `Dynamic`/`Computed`, `Mode` variance, and float→int narrowing — become
/// [`UnknownReason`] so a strict policy can see them. `Number` int→float
/// widening and equal primitive variants are provably compatible (nothing
/// pushed).
fn collect_pair(
    key: &FieldKey,
    producer_field: &Field,
    consumer_field: &Field,
    strict: bool,
    acc: &mut Explain,
) {
    // Dynamic/Computed on either side: loader/expression-backed, concrete shape
    // unknown until runtime — not statically provable in either direction.
    if matches!(producer_field, Field::Dynamic(_) | Field::Computed(_))
        || matches!(consumer_field, Field::Dynamic(_) | Field::Computed(_))
    {
        acc.unknown
            .push(UnknownReason::DynamicLoaderBacked { key: key.clone() });
        return;
    }

    match (producer_field, consumer_field) {
        (Field::File(p), Field::File(c)) => {
            if p.multiple != c.multiple {
                acc.incompat.push(SchemaIncompat::CardinalityMismatch {
                    key: key.clone(),
                    producer_multiple: p.multiple,
                    consumer_multiple: c.multiple,
                });
            }
        },
        (Field::Select(p), Field::Select(c)) => {
            if p.multiple != c.multiple {
                acc.incompat.push(SchemaIncompat::CardinalityMismatch {
                    key: key.clone(),
                    producer_multiple: p.multiple,
                    consumer_multiple: c.multiple,
                });
            }
        },
        (Field::Object(producer_obj), Field::Object(consumer_obj)) => {
            let mut sub = Explain::default();
            collect_fields(&producer_obj.fields, &consumer_obj.fields, strict, &mut sub);
            acc.wrap_nested(key, sub);
        },
        (Field::List(producer_list), Field::List(consumer_list)) => {
            match (&producer_list.item, &consumer_list.item) {
                // Either side has no typed item schema — Any escape (provably Yes).
                (None, _) | (_, None) => {},
                (Some(producer_item), Some(consumer_item)) => {
                    // The nested context is the list field `key`, but the inner
                    // incompatibility is labeled with the *item's* key (not the
                    // list key — that conflation was the old list-item-key bug).
                    let mut sub = Explain::default();
                    collect_pair(
                        consumer_item.key(),
                        producer_item,
                        consumer_item,
                        strict,
                        &mut sub,
                    );
                    acc.wrap_nested(key, sub);
                },
            }
        },
        (Field::Number(p), Field::Number(c)) => {
            // int→float widening is safe (provably Yes). float→int narrowing may
            // lose precision — but a float producer could still only ever emit
            // integral values, so it is not provably wrong: Unknown, not No.
            if !p.integer && c.integer {
                acc.unknown
                    .push(UnknownReason::NumberWidening { key: key.clone() });
            }
        },
        (Field::Mode(_), Field::Mode(_)) => {
            // Sum-type variance is not modeled (it runs opposite to record
            // width-subtyping) — neither proven compatible nor refuted.
            acc.unknown
                .push(UnknownReason::ModeVariance { key: key.clone() });
        },
        // All other pairs: same type_name = compatible; different = type mismatch.
        _ => {
            if producer_field.type_name() != consumer_field.type_name() {
                acc.incompat.push(SchemaIncompat::FieldTypeMismatch {
                    key: key.clone(),
                    producer: producer_field.type_name(),
                    consumer: consumer_field.type_name(),
                });
            }
        },
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Field;

    fn fk(s: &str) -> FieldKey {
        FieldKey::new(s).unwrap()
    }

    /// Gradual, kind-blind slice check: an empty producer slice is the `Any`
    /// escape. This is **test-only** — production code calls
    /// [`is_assignable_schema`], which honors the `SchemaKind` Top/Bottom split.
    /// Retained here to exercise the shared per-field matching logic (type
    /// mismatch, cardinality, nesting) and the gradual empty-producer escape
    /// directly on `&[Field]` without building a `ValidSchema` per case. Mirrors
    /// the binary mapping: `Yes`/`Unknown` ⇒ `Ok`, `No` ⇒ first incompatibility.
    fn is_assignable(producer: &[Field], consumer: &[Field]) -> Result<(), SchemaIncompat> {
        match explain_slice(producer, consumer, false) {
            Assignability::Yes | Assignability::Unknown(_) => Ok(()),
            Assignability::No(incompats) => match incompats.into_iter().next() {
                Some(first) => Err(first),
                None => Ok(()),
            },
        }
    }

    /// Test-only collect-all over raw slices, with explicit `strict` control.
    fn explain_slice(producer: &[Field], consumer: &[Field], strict: bool) -> Assignability {
        let mut acc = Explain::default();
        collect_fields(producer, consumer, strict, &mut acc);
        acc.into_verdict()
    }

    // ── Compatible: producer has all required consumer fields + extra ──────

    #[test]
    fn compatible_with_extra_producer_field() {
        let producer = [
            Field::string(fk("name")).required().into(),
            Field::number(fk("score")).into(),
            Field::boolean(fk("extra")).into(), // producer-only, ignored
        ];
        let consumer = [
            Field::string(fk("name")).required().into(),
            Field::number(fk("score")).into(),
        ];
        assert_eq!(is_assignable(&producer, &consumer), Ok(()));
    }

    // ── Missing hard-required field ────────────────────────────────────────

    #[test]
    fn missing_required_field_returns_error() {
        let producer = [Field::number(fk("score")).into()];
        let consumer = [
            Field::string(fk("name")).required().into(),
            Field::number(fk("score")).into(),
        ];
        assert_eq!(
            is_assignable(&producer, &consumer),
            Err(SchemaIncompat::MissingRequiredField { key: fk("name") })
        );
    }

    // ── Type mismatch on shared field ──────────────────────────────────────

    #[test]
    fn type_mismatch_on_shared_field_returns_error() {
        let producer = [Field::number(fk("value")).required().into()];
        let consumer = [Field::string(fk("value")).required().into()];
        assert_eq!(
            is_assignable(&producer, &consumer),
            Err(SchemaIncompat::FieldTypeMismatch {
                key: fk("value"),
                producer: "number",
                consumer: "string",
            })
        );
    }

    // ── Optional consumer field absent from producer ───────────────────────

    #[test]
    fn optional_consumer_field_absent_is_ok() {
        let producer = [Field::string(fk("name")).required().into()];
        let consumer = [
            Field::string(fk("name")).required().into(),
            Field::number(fk("optional_score")).into(), // optional, absent in producer
        ];
        assert_eq!(is_assignable(&producer, &consumer), Ok(()));
    }

    // ── When-required consumer field absent treated as optional ───────────

    #[test]
    fn when_required_absent_is_ok() {
        use crate::Rule;
        use nebula_validator::Predicate;
        use serde_json::json;

        let rule = Rule::predicate(Predicate::eq("mode", json!("advanced")).unwrap());
        let producer = [Field::string(fk("name")).required().into()];
        let consumer = [
            Field::string(fk("name")).required().into(),
            Field::string(fk("advanced_opt")).required_when(rule).into(),
        ];
        assert_eq!(is_assignable(&producer, &consumer), Ok(()));
    }

    // ── Nested Object: consumer-object requires a field producer-object lacks

    #[test]
    fn nested_object_missing_required_returns_error() {
        let producer = [Field::object(fk("config"))
            .add(Field::string(fk("host")).required())
            // "port" absent from producer's config
            .into()];
        let consumer = [Field::object(fk("config"))
            .add(Field::string(fk("host")).required())
            .add(Field::number(fk("port")).required())
            .into()];
        // Both outer fields are Object — recurse; inner check finds "port"
        // missing → NestedIncompat wrapping MissingRequiredField.
        assert_eq!(
            is_assignable(&producer, &consumer),
            Err(SchemaIncompat::NestedIncompat {
                key: fk("config"),
                inner: Box::new(SchemaIncompat::MissingRequiredField { key: fk("port") }),
            })
        );
    }

    // ── Nested Object: inner scalar field type mismatch ───────────────────

    #[test]
    fn nested_object_field_type_mismatch_returns_nested_incompat() {
        let producer = [Field::object(fk("config"))
            .add(Field::number(fk("port")).required()) // number in producer
            .into()];
        let consumer = [Field::object(fk("config"))
            .add(Field::string(fk("port")).required()) // string in consumer
            .into()];
        assert_eq!(
            is_assignable(&producer, &consumer),
            Err(SchemaIncompat::NestedIncompat {
                key: fk("config"),
                inner: Box::new(SchemaIncompat::FieldTypeMismatch {
                    key: fk("port"),
                    producer: "number",
                    consumer: "string",
                }),
            })
        );
    }

    // ── Nested Object: fully nested-compatible ─────────────────────────────

    #[test]
    fn nested_object_fully_compatible_is_ok() {
        let producer = [Field::object(fk("config"))
            .add(Field::string(fk("host")).required())
            .add(Field::number(fk("port")).required())
            .into()];
        let consumer = [Field::object(fk("config"))
            .add(Field::string(fk("host")).required())
            .add(Field::number(fk("port")).required())
            .into()];
        assert_eq!(is_assignable(&producer, &consumer), Ok(()));
    }

    // ── List: compatible item types ────────────────────────────────────────

    #[test]
    fn list_compatible_item_types_is_ok() {
        let producer = [Field::list(fk("tags"))
            .item(Field::string(fk("tag")))
            .into()];
        let consumer = [Field::list(fk("tags"))
            .item(Field::string(fk("tag")))
            .into()];
        assert_eq!(is_assignable(&producer, &consumer), Ok(()));
    }

    // ── List: mismatched item types → NestedIncompat ───────────────────────

    #[test]
    fn list_mismatched_item_types_returns_nested_incompat() {
        let producer = [Field::list(fk("values"))
            .item(Field::string(fk("item")))
            .required()
            .into()];
        let consumer = [Field::list(fk("values"))
            .item(Field::number(fk("item")))
            .required()
            .into()];
        // Outer NestedIncompat is keyed by the list field (`values`); the inner
        // mismatch is keyed by the *item* (`item`), not the list key.
        assert_eq!(
            is_assignable(&producer, &consumer),
            Err(SchemaIncompat::NestedIncompat {
                key: fk("values"),
                inner: Box::new(SchemaIncompat::FieldTypeMismatch {
                    key: fk("item"),
                    producer: "string",
                    consumer: "number",
                }),
            })
        );
    }

    // ── Any escape: empty producer ─────────────────────────────────────────

    #[test]
    fn empty_producer_satisfies_typed_consumer() {
        let consumer = [Field::string(fk("name")).required().into()];
        assert_eq!(is_assignable(&[], &consumer), Ok(()));
    }

    // ── Any escape: Dynamic producer field vs typed required consumer ──────

    #[test]
    fn dynamic_producer_field_satisfies_typed_required_consumer() {
        let producer = [Field::dynamic(fk("name")).into()];
        let consumer = [Field::string(fk("name")).required().into()];
        assert_eq!(is_assignable(&producer, &consumer), Ok(()));
    }

    // ── Any escape: empty consumer accepts everything ──────────────────────

    #[test]
    fn empty_consumer_accepts_any_producer() {
        let producer = [Field::string(fk("name")).required().into()];
        assert_eq!(is_assignable(&producer, &[]), Ok(()));
    }

    // ── Notice consumer field is ignored ──────────────────────────────────

    #[test]
    fn notice_consumer_field_ignored() {
        let producer = [Field::string(fk("name")).required().into()];
        let consumer = [
            Field::string(fk("name")).required().into(),
            Field::notice(fk("tip")).into(), // absent in producer, but ignored
        ];
        assert_eq!(is_assignable(&producer, &consumer), Ok(()));
    }

    // ── Extra producer fields ignored (width subtyping) ───────────────────

    #[test]
    fn extra_producer_fields_ignored() {
        let producer = [
            Field::string(fk("name")).required().into(),
            Field::number(fk("extra_a")).into(),
            Field::boolean(fk("extra_b")).into(),
        ];
        let consumer = [Field::string(fk("name")).required().into()];
        assert_eq!(is_assignable(&producer, &consumer), Ok(()));
    }

    // ── Cardinality: File single→multiple is a mismatch ───────────────────

    #[test]
    fn file_single_to_multiple_returns_cardinality_mismatch() {
        // producer: single file; consumer expects multiple files
        let producer = [Field::file(fk("attachment")).required().into()];
        let consumer = [Field::file(fk("attachment")).multiple().required().into()];
        assert_eq!(
            is_assignable(&producer, &consumer),
            Err(SchemaIncompat::CardinalityMismatch {
                key: fk("attachment"),
                producer_multiple: false,
                consumer_multiple: true,
            })
        );
    }

    // ── Cardinality: File multiple→multiple is compatible ─────────────────

    #[test]
    fn file_multiple_to_multiple_is_ok() {
        let producer = [Field::file(fk("attachment")).multiple().required().into()];
        let consumer = [Field::file(fk("attachment")).multiple().required().into()];
        assert_eq!(is_assignable(&producer, &consumer), Ok(()));
    }

    // ── Cardinality: Select cardinality mismatch ──────────────────────────

    #[test]
    fn select_cardinality_mismatch_returns_error() {
        // producer: multi-select; consumer: single-select
        let producer = [Field::select(fk("tags")).multiple().required().into()];
        let consumer = [Field::select(fk("tags")).required().into()];
        assert_eq!(
            is_assignable(&producer, &consumer),
            Err(SchemaIncompat::CardinalityMismatch {
                key: fk("tags"),
                producer_multiple: true,
                consumer_multiple: false,
            })
        );
    }

    // ── Cardinality: Select same cardinality is compatible ────────────────

    #[test]
    fn select_same_cardinality_is_ok() {
        let producer = [Field::select(fk("tags")).multiple().required().into()];
        let consumer = [Field::select(fk("tags")).multiple().required().into()];
        assert_eq!(is_assignable(&producer, &consumer), Ok(()));
    }

    // ── Kind-aware entry point (`is_assignable_schema`): Top/Bottom split ──────

    /// Build a single-required-field record `ValidSchema`.
    fn required_record(key: &str) -> ValidSchema {
        crate::Schema::builder()
            .add(Field::string(fk(key)).required())
            .build()
            .unwrap()
    }

    /// The defining fix: an empty **record** producer (`()`) does NOT satisfy a
    /// consumer that hard-requires a field — unlike the gradual `Any`, an empty
    /// record provably emits nothing.
    #[test]
    fn empty_record_producer_does_not_satisfy_required_consumer() {
        let producer = ValidSchema::empty(); // Record, zero fields
        let consumer = required_record("name");
        assert_eq!(
            is_assignable_schema(&producer, &consumer),
            Err(SchemaIncompat::MissingRequiredField { key: fk("name") }),
        );
    }

    /// Contrast: a gradual `Any` producer DOES satisfy the same required
    /// consumer — the escape hatch the slice-level check could not distinguish.
    #[test]
    fn any_producer_satisfies_required_consumer() {
        let producer = ValidSchema::any();
        let consumer = required_record("name");
        assert_eq!(is_assignable_schema(&producer, &consumer), Ok(()));
    }

    /// An `Any` consumer accepts any producer (it requires nothing).
    #[test]
    fn typed_producer_satisfies_any_consumer() {
        let producer = required_record("name");
        let consumer = ValidSchema::any();
        assert_eq!(is_assignable_schema(&producer, &consumer), Ok(()));
    }

    /// An empty **record** consumer accepts any producer (requires nothing) —
    /// the consumer-side escape is preserved in strict mode.
    #[test]
    fn empty_record_consumer_accepts_typed_producer() {
        let producer = required_record("name");
        let consumer = ValidSchema::empty();
        assert_eq!(is_assignable_schema(&producer, &consumer), Ok(()));
    }

    /// Two compatible concrete records pass through the kind-aware entry point,
    /// matching the slice-level result (no behavior drift on the typed path).
    #[test]
    fn compatible_records_via_schema_entry_is_ok() {
        let producer = crate::Schema::builder()
            .add(Field::string(fk("name")).required())
            .add(Field::number(fk("extra")))
            .build()
            .unwrap();
        let consumer = required_record("name");
        assert_eq!(is_assignable_schema(&producer, &consumer), Ok(()));
    }

    /// Two incompatible concrete records still produce the same first
    /// incompatibility through the kind-aware entry point.
    #[test]
    fn incompatible_records_via_schema_entry_returns_error() {
        let producer = required_record("name");
        let consumer = required_record("other");
        assert_eq!(
            is_assignable_schema(&producer, &consumer),
            Err(SchemaIncompat::MissingRequiredField { key: fk("other") }),
        );
    }

    /// Both sides `Any` short-circuits to `Ok` via the leading `||` term. Guards
    /// against the condition being mistyped as `&&` (which would fall through to
    /// the strict record path — masked here only because both field slices are
    /// empty, so this test pins the intended both-`Any` contract explicitly).
    #[test]
    fn both_any_schemas_are_compatible() {
        assert_eq!(
            is_assignable_schema(&ValidSchema::any(), &ValidSchema::any()),
            Ok(()),
        );
    }

    // ── Strict-mode recursion: `strict` must reach nested Object / List leaves ──
    // These drive the collect-all core via `explain_slice` and contrast strict
    // vs gradual at depth, so a dropped `strict` argument in the Object/List
    // recursion arms goes RED.

    /// An empty nested-object producer does NOT satisfy a consumer whose nested
    /// object hard-requires a field — under strict mode, one level deep. Gradual
    /// mode escapes it, proving the assertion is `strict`-specific.
    #[test]
    fn strict_rejects_empty_nested_object_producer() {
        let producer = [Field::object(fk("config")).into()]; // empty inner object
        let consumer = [Field::object(fk("config"))
            .add(Field::string(fk("host")).required())
            .into()];

        assert_eq!(
            explain_slice(&producer, &consumer, true),
            Assignability::No(vec![SchemaIncompat::NestedIncompat {
                key: fk("config"),
                inner: Box::new(SchemaIncompat::MissingRequiredField { key: fk("host") }),
            }]),
            "strict mode must recurse into the empty producer object and fail the required field"
        );
        assert_eq!(
            explain_slice(&producer, &consumer, false),
            Assignability::Yes,
            "gradual mode escapes the empty nested producer — confirms the strict flag is what bites"
        );
    }

    /// A list whose item is an empty object does NOT satisfy a consumer list
    /// whose item object hard-requires a field — strict propagates List → item →
    /// Object. Gradual mode escapes it. The inner context key is the *item*
    /// (`item`), the outer is the list (`items`).
    #[test]
    fn strict_rejects_empty_nested_list_item_producer() {
        let producer = [Field::list(fk("items"))
            .item(Field::object(fk("item")))
            .into()];
        let consumer = [Field::list(fk("items"))
            .item(Field::object(fk("item")).add(Field::string(fk("id")).required()))
            .into()];

        assert_eq!(
            explain_slice(&producer, &consumer, true),
            Assignability::No(vec![SchemaIncompat::NestedIncompat {
                key: fk("items"),
                inner: Box::new(SchemaIncompat::NestedIncompat {
                    key: fk("item"),
                    inner: Box::new(SchemaIncompat::MissingRequiredField { key: fk("id") }),
                }),
            }]),
            "strict mode must recurse List -> item -> Object and fail the required field"
        );
        assert_eq!(
            explain_slice(&producer, &consumer, false),
            Assignability::Yes,
            "gradual mode escapes the empty list-item object — confirms strict propagation"
        );
    }

    // ── Ternary `explain_assignable`: Yes / No(all) / Unknown(reasons) ─────────

    /// `explain_assignable` collects ALL incompatibilities, not just the first —
    /// the property that makes it a CI/diagnostics channel rather than a gate.
    #[test]
    fn explain_collects_all_incompatibilities() {
        let producer = required_record("present");
        let consumer = crate::Schema::builder()
            .add(Field::string(fk("missing_a")).required())
            .add(Field::string(fk("missing_b")).required())
            .build()
            .unwrap();
        match explain_assignable(&producer, &consumer) {
            Assignability::No(incompats) => {
                assert_eq!(
                    incompats.len(),
                    2,
                    "both missing required fields are reported"
                );
                assert!(incompats.contains(&SchemaIncompat::MissingRequiredField {
                    key: fk("missing_a")
                }));
                assert!(incompats.contains(&SchemaIncompat::MissingRequiredField {
                    key: fk("missing_b")
                }));
            },
            other => panic!("expected No(2), got {other:?}"),
        }
    }

    /// An `Any` producer is `Unknown(OpaqueProducer)` — not provable — even
    /// though the binary check passes it (gradual escape).
    #[test]
    fn explain_any_producer_is_unknown_opaque() {
        let consumer = required_record("name");
        assert_eq!(
            explain_assignable(&ValidSchema::any(), &consumer),
            Assignability::Unknown(vec![UnknownReason::OpaqueProducer]),
        );
        // The binary view still passes it.
        assert!(is_assignable_schema(&ValidSchema::any(), &consumer).is_ok());
    }

    /// An `Any` consumer is provably `Yes` (it accepts anything).
    #[test]
    fn explain_any_consumer_is_yes() {
        let producer = required_record("name");
        assert_eq!(
            explain_assignable(&producer, &ValidSchema::any()),
            Assignability::Yes,
        );
    }

    /// A `Dynamic` producer field is `Unknown(DynamicLoaderBacked)` in explain,
    /// yet `Ok` in the binary check (the lofty per-field gradual escape).
    #[test]
    fn explain_dynamic_field_is_unknown_not_ok() {
        let producer = crate::Schema::builder()
            .add(Field::dynamic(fk("name")))
            .build()
            .unwrap();
        let consumer = required_record("name");
        assert_eq!(
            explain_assignable(&producer, &consumer),
            Assignability::Unknown(vec![UnknownReason::DynamicLoaderBacked { key: fk("name") }]),
        );
        assert!(is_assignable_schema(&producer, &consumer).is_ok());
    }

    /// Number int→float widens (provably `Yes`); float→int narrows
    /// (`Unknown(NumberWidening)`, not a hard `No`).
    #[test]
    fn explain_number_widening_is_directional() {
        let int_producer = crate::Schema::builder()
            .add(Field::number(fk("n")).integer())
            .build()
            .unwrap();
        let float_consumer = crate::Schema::builder()
            .add(Field::number(fk("n")))
            .build()
            .unwrap();
        // int -> float: safe widening.
        assert_eq!(
            explain_assignable(&int_producer, &float_consumer),
            Assignability::Yes,
        );
        // float -> int: possible precision loss, undecidable.
        assert_eq!(
            explain_assignable(&float_consumer, &int_producer),
            Assignability::Unknown(vec![UnknownReason::NumberWidening { key: fk("n") }]),
        );
        // Both pass the binary check (Unknown ⇒ Ok).
        assert!(is_assignable_schema(&float_consumer, &int_producer).is_ok());
    }

    /// A definite incompatibility dominates an undecidable one: `No` ▸ `Unknown`.
    #[test]
    fn explain_no_dominates_unknown() {
        let producer = crate::Schema::builder()
            .add(Field::dynamic(fk("d")))
            .build()
            .unwrap();
        let consumer = crate::Schema::builder()
            .add(Field::dynamic(fk("d")))
            .add(Field::string(fk("required_missing")).required())
            .build()
            .unwrap();
        match explain_assignable(&producer, &consumer) {
            Assignability::No(v) => assert_eq!(
                v,
                vec![SchemaIncompat::MissingRequiredField {
                    key: fk("required_missing")
                }]
            ),
            other => panic!("a provable incompatibility must dominate Unknown, got {other:?}"),
        }
    }

    /// A `Mode`-vs-`Mode` pair is `Unknown(ModeVariance)` (sum-type variance is
    /// not modeled) yet passes the binary check. The one reclassified leniency
    /// otherwise lacking an oracle.
    #[test]
    fn explain_mode_variance_is_unknown() {
        let mode_schema = || {
            crate::Schema::builder()
                .add(Field::mode(fk("m")).variant("v", "V", Field::string(fk("x"))))
                .build()
                .unwrap()
        };
        let producer = mode_schema();
        let consumer = mode_schema();
        assert_eq!(
            explain_assignable(&producer, &consumer),
            Assignability::Unknown(vec![UnknownReason::ModeVariance { key: fk("m") }]),
        );
        assert!(is_assignable_schema(&producer, &consumer).is_ok());
    }

    /// An undecidable field nested inside an `Object` keeps its path: the reason
    /// is `NestedUnknown { config, DynamicLoaderBacked { d } }`, not a bare
    /// `DynamicLoaderBacked { d }` that a sibling object's `d` could alias.
    #[test]
    fn explain_nested_object_unknown_preserves_path() {
        let producer = crate::Schema::builder()
            .add(Field::object(fk("config")).add(Field::dynamic(fk("d"))))
            .build()
            .unwrap();
        // Consumer's nested `d` is optional, so the only finding is the Unknown
        // (no MissingRequiredField to dominate it).
        let consumer = crate::Schema::builder()
            .add(Field::object(fk("config")).add(Field::string(fk("d"))))
            .build()
            .unwrap();
        assert_eq!(
            explain_assignable(&producer, &consumer),
            Assignability::Unknown(vec![UnknownReason::NestedUnknown {
                key: fk("config"),
                inner: Box::new(UnknownReason::DynamicLoaderBacked { key: fk("d") }),
            }]),
        );
    }

    /// An undecidable field nested inside a `List` item keeps both the list key
    /// and the item key: `NestedUnknown { items, NestedUnknown { item, .. } }`.
    #[test]
    fn explain_nested_list_unknown_preserves_path() {
        let producer = crate::Schema::builder()
            .add(
                Field::list(fk("items"))
                    .item(Field::object(fk("item")).add(Field::dynamic(fk("d")))),
            )
            .build()
            .unwrap();
        let consumer = crate::Schema::builder()
            .add(
                Field::list(fk("items"))
                    .item(Field::object(fk("item")).add(Field::string(fk("d")))),
            )
            .build()
            .unwrap();
        assert_eq!(
            explain_assignable(&producer, &consumer),
            Assignability::Unknown(vec![UnknownReason::NestedUnknown {
                key: fk("items"),
                inner: Box::new(UnknownReason::NestedUnknown {
                    key: fk("item"),
                    inner: Box::new(UnknownReason::DynamicLoaderBacked { key: fk("d") }),
                }),
            }]),
        );
    }

    /// `No` dominates `Unknown` even when they arise in DIFFERENT nested
    /// containers: a missing-required in object `a` wins over a dynamic field in
    /// sibling object `b`, and the `Unknown` is dropped.
    #[test]
    fn explain_no_dominates_unknown_across_containers() {
        let producer = crate::Schema::builder()
            .add(Field::object(fk("a"))) // empty — can't satisfy a's required child
            .add(Field::object(fk("b")).add(Field::dynamic(fk("d"))))
            .build()
            .unwrap();
        let consumer = crate::Schema::builder()
            .add(Field::object(fk("a")).add(Field::string(fk("need")).required()))
            .add(Field::object(fk("b")).add(Field::string(fk("d"))))
            .build()
            .unwrap();
        assert_eq!(
            explain_assignable(&producer, &consumer),
            Assignability::No(vec![SchemaIncompat::NestedIncompat {
                key: fk("a"),
                inner: Box::new(SchemaIncompat::MissingRequiredField { key: fk("need") }),
            }]),
            "No dominates across containers; the sibling Unknown is dropped"
        );
    }

    #[test]
    fn unknown_reason_display_is_human_readable() {
        assert_eq!(
            UnknownReason::OpaqueProducer.to_string(),
            "producer schema is an opaque `Any` (shape unknown)"
        );
        assert_eq!(
            UnknownReason::DynamicLoaderBacked { key: fk("tok") }.to_string(),
            "field `tok` is dynamic/computed (resolved at runtime)"
        );
        // Nested reasons render their path prefix.
        assert_eq!(
            UnknownReason::NestedUnknown {
                key: fk("cfg"),
                inner: Box::new(UnknownReason::ModeVariance { key: fk("m") }),
            }
            .to_string(),
            "in field `cfg`: field `m` is a `Mode` sum type (variance not modeled)"
        );
    }
}
