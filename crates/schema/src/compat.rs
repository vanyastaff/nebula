//! Schema-compatibility check: structural width-subtyping (TypeDAG T1).
//!
//! The kernel of the ADR-0100 connection type-check. Called by the workflow
//! per-edge validator (T3) to decide whether a producer node's `Output` schema
//! is assignable where a consumer node's `Input` schema is expected.
//!
//! The public entry point is [`explain_assignable`], taking direction-typed
//! output/input schemas and retaining `Yes`, `No`, and `Unknown` as distinct
//! verdicts. Metadata revision compatibility remains a separate contract.

use crate::{
    Field, FieldKey, InputSchema, OutputSchema, RequiredMode, RootShape, ScalarKind, ScalarSchema,
    SchemaKind, SerdeTagging, ValidSchema, field::ModeField,
};
use nebula_validator::{DiagnosticDisclosure, ValueRule};
use serde_json::{Number, Value};

// ── Public types ─────────────────────────────────────────────────────────────

/// Why a producer schema is not assignable to a consumer schema.
///
/// Carried by [`Assignability::No`] when the structural check finds a definite
/// conflict. Findings retain depth-first, consumer-field order.
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
    /// A `Mode` (tagged-union) field is present on both sides, but the producer
    /// declares a variant the consumer does not accept. Sum-type subtyping is the
    /// dual of record width-subtyping: the producer may emit *any* of its
    /// variants at runtime, so every producer variant must have a counterpart in
    /// the consumer — otherwise the consumer would receive a case it cannot
    /// handle. (The reverse is fine: a consumer that accepts *more* variants than
    /// the producer can emit is still satisfied.)
    #[error("field `{key}` mode variant `{variant}` is not accepted by the consumer")]
    UnhandledVariant {
        /// Key of the `Mode` field.
        key: FieldKey,
        /// The producer variant key the consumer does not declare.
        variant: String,
    },
    /// The two concrete root shapes differ: scalar, record, or tagged union.
    /// An [`Any`](SchemaKind::Any) on either side is not a kind mismatch.
    #[error("schema kind mismatch: producer is {producer:?}, consumer expects {consumer:?}")]
    KindMismatch {
        /// The producer schema's kind.
        producer: SchemaKind,
        /// The consumer schema's kind.
        consumer: SchemaKind,
    },
    /// Two scalar roots have disjoint JSON kinds. Integer/number overlap is
    /// handled separately, never reported as a kind mismatch.
    #[error("scalar kind mismatch: producer is {producer:?}, consumer expects {consumer:?}")]
    ScalarKindMismatch {
        /// Producer scalar kind.
        producer: ScalarKind,
        /// Consumer scalar kind.
        consumer: ScalarKind,
    },
    /// The inclusive numeric domains share no value. Bounds are intentionally
    /// omitted from the diagnostic payload.
    #[error("producer and consumer scalar numeric domains are disjoint")]
    ScalarBoundsDisjoint,
    /// Both schemas are tagged [`Union`](SchemaKind::Union)s, but their serde
    /// tagging differs (e.g. external vs adjacent). The variant keys may match,
    /// yet the on-the-wire shapes do not (`{"V": payload}` vs
    /// `{"<tag>": "V", "<content>": payload}`) and are not interconvertible, so the
    /// consumer cannot deserialize the producer's output. `serde_tagging` is part
    /// of schema identity (see the `ValidSchema` `PartialEq`).
    #[error("union serde tagging mismatch: producer {producer:?}, consumer expects {consumer:?}")]
    UnionTaggingMismatch {
        /// The producer union's serde tagging (`None` only on a malformed union).
        producer: Option<SerdeTagging>,
        /// The consumer union's serde tagging.
        consumer: Option<SerdeTagging>,
    },
}

// ── Public entry point ───────────────────────────────────────────────────────

/// Direction-typed structural assignability of a producer's output to a consumer's input.
///
/// This returns the complete three-valued verdict, never a binary success for
/// an unproven edge. A strict admission policy accepts only [`Assignability::Yes`];
/// any gradual policy must explicitly handle [`Assignability::Unknown`].
/// Swapping the [`OutputSchema`] and [`InputSchema`] arguments is a type error.
///
/// # Root Shapes
///
/// An `Any` consumer accepts every producer. An `Any` producer against a
/// concrete consumer is unknown. Empty records have object shape, not universal
/// shape: they accept other records by width-subtyping, not scalars or unions.
/// Scalar domains compare by kind and exact numeric bounds. Additional
/// consumer root rules must be proven; otherwise the verdict is unknown.
/// Identical context-free value rules already on the producer are proven
/// without execution. Context-dependent or other unanalysed rules stay unknown.
///
/// # Field Contracts
///
/// Record matching retains the established structural field contract:
/// required consumer fields must exist, extra producer fields are ignored,
/// and display-only notices are ignored. Nested objects and list items recurse.
/// File/select cardinality must match. Every producer mode variant must exist
/// in the consumer, with covariant payloads; root unions also require matching
/// serde tagging. Dynamic/computed and unrecognized fields are unknown.
/// Number fields widen integer to number, but narrowing is unknown.
///
/// Findings retain depth-first consumer-field order (producer-variant order
/// within a mode). A definite incompatibility takes precedence over uncertainty.
/// This is an edge relation, not metadata identity or revision compatibility.
///
/// # Examples
///
/// ```rust
/// use nebula_schema::{
///     Assignability, Field, InputSchema, OutputSchema, Schema, UnknownReason,
///     ValidSchema, explain_assignable, field_key,
/// };
///
/// let consumer = InputSchema::new(
///     Schema::builder()
///         .add(Field::string(field_key!("name")).required())
///         .build()?,
/// );
/// let producer = OutputSchema::new(consumer.as_schema().clone());
/// assert_eq!(explain_assignable(&producer, &consumer), Assignability::Yes);
/// assert_eq!(
///     explain_assignable(&OutputSchema::new(ValidSchema::any()), &consumer),
///     Assignability::Unknown(vec![UnknownReason::OpaqueProducer]),
/// );
/// # Ok::<(), nebula_schema::ValidationReport>(())
/// ```
#[must_use]
#[tracing::instrument(name = "schema.assignability", skip_all, fields(
    producer_kind = ?producer.as_schema().kind(),
    consumer_kind = ?consumer.as_schema().kind(),
))]
pub fn explain_assignable(producer: &OutputSchema, consumer: &InputSchema) -> Assignability {
    explain_assignable_core(producer.as_schema(), consumer.as_schema())
}

/// Field-granularity counterpart to [`explain_assignable`]: is `producer_leaf`
/// assignable where `consumer_leaf` is expected, when a caller has two
/// individual resolved [`Field`]s in hand rather than two whole schemas (e.g.
/// `nebula-workflow`'s per-field `Reference`-parameter check, which resolves a
/// single producer field and a single consumer field, not their enclosing
/// schemas).
///
/// Internally wraps each field into a synthetic one-field schema (both under
/// `consumer_leaf`'s key, so the two pair up — see the crate-private
/// `ValidSchema::single_field`), tags them `Output`/`Input`, and runs the
/// exact same [`explain_assignable`] the schema-level check uses. Those
/// synthetic schemas are built, compared, and dropped entirely inside this
/// function — never handed back to the caller, so the bypassed-lint,
/// bypassed-index construction `ValidSchema::single_field` performs never
/// crosses a crate boundary as a value someone could mistake for a
/// fully-built schema.
///
/// A [`Field::Unknown`] leaf is handled BEFORE that synthetic-schema
/// machinery runs, not after: `ValidSchema::single_field`'s re-keying
/// (`rekeyed`) cannot rewrite an `Unknown` field's private key (see its own
/// doc comment), so a `Field::Unknown` producer or consumer leaf whose
/// original key differs from the other side's key would fail to pair up
/// under the shared synthetic key — surfacing as a spurious
/// `No(MissingRequiredField)` (required consumer field) or a spurious `Yes`
/// (optional consumer field), rather than the correct "this version cannot
/// reason about an unrecognized field kind" verdict.
#[must_use]
pub fn explain_field_assignable(producer_leaf: &Field, consumer_leaf: &Field) -> Assignability {
    // `Field::Unknown` is opaque on either side (mirrors `collect_pair`'s own
    // same-key `Field::Unknown` handling a few lines below) — decide this
    // before building any synthetic schema, since the re-key that machinery
    // relies on cannot reach an `Unknown` field's key.
    if matches!(producer_leaf, Field::Unknown(_)) || matches!(consumer_leaf, Field::Unknown(_)) {
        return Assignability::Unknown(vec![UnknownReason::OpaqueFieldKind {
            key: consumer_leaf.key().clone(),
        }]);
    }

    let key = consumer_leaf.key().clone();
    let producer = OutputSchema::new(ValidSchema::single_field(
        key.clone(),
        producer_leaf.clone(),
    ));
    let consumer = InputSchema::new(ValidSchema::single_field(key, consumer_leaf.clone()));
    explain_assignable(&producer, &consumer)
}

/// Root-to-field counterpart to [`explain_assignable`]: is a producer's
/// complete concrete root assignable where one consumer parameter field is
/// expected?
///
/// This is the compatibility operation for a root [`crate::ValuePath`]
/// returned as [`crate::PathWalk::ResolvedRoot`]. Record roots compare against
/// object fields by width subtyping, while scalar roots compare against the
/// corresponding scalar field kind. Consumer field rules must either be
/// context-free value rules already present on the producer root or the result
/// is [`Assignability::Unknown`]. Callers must retain the reference walk's
/// fail-open handling for `Any`, unions, and opaque consumer fields.
#[must_use]
#[tracing::instrument(name = "schema.root_field_assignability", skip_all, fields(
    producer_kind = ?producer_root.as_schema().kind(),
    consumer_kind = consumer_field.type_name(),
))]
pub fn explain_root_field_assignable(
    producer_root: &OutputSchema,
    consumer_field: &Field,
) -> Assignability {
    let producer_schema = producer_root.as_schema();
    let mut findings = Explain::default();
    match producer_schema.root_shape() {
        RootShape::Any | RootShape::Union(_) => {
            return Assignability::Unknown(vec![UnknownReason::OpaqueProducer]);
        },
        RootShape::Record(producer_record) => match consumer_field {
            Field::Object(consumer_object) => collect_fields(
                producer_record.fields(),
                &consumer_object.fields,
                true,
                &mut findings,
            ),
            _ => {
                return Assignability::No(vec![SchemaIncompat::FieldTypeMismatch {
                    key: consumer_field.key().clone(),
                    producer: "object",
                    consumer: consumer_field.type_name(),
                }]);
            },
        },
        RootShape::Scalar(producer_scalar) => {
            collect_root_scalar_field(producer_scalar, consumer_field, &mut findings);
        },
    }
    if !consumer_field.rules().iter().all(|rule| {
        matches!(rule.view(), nebula_validator::RuleView::Value(_))
            && producer_schema.root_rules().contains(rule)
    }) {
        findings.unknown.push(UnknownReason::UnprovenRootRules);
    }
    findings.into_verdict()
}

fn collect_root_scalar_field(producer: &ScalarSchema, consumer: &Field, findings: &mut Explain) {
    let is_compatible = matches!(
        (producer.kind(), consumer),
        (ScalarKind::String, Field::String(_))
            | (ScalarKind::Boolean, Field::Boolean(_))
            | (ScalarKind::Integer, Field::Number(_))
            | (
                ScalarKind::Number,
                Field::Number(crate::NumberField { integer: false, .. })
            )
    );
    if is_compatible {
        return;
    }
    if matches!(
        (producer.kind(), consumer),
        (
            ScalarKind::Number,
            Field::Number(crate::NumberField { integer: true, .. })
        )
    ) {
        findings.unknown.push(UnknownReason::NumberWidening {
            key: consumer.key().clone(),
        });
        return;
    }
    let producer_type = match producer.kind() {
        ScalarKind::Null => "null",
        ScalarKind::Boolean => "boolean",
        ScalarKind::String => "string",
        ScalarKind::Integer | ScalarKind::Number => "number",
    };
    findings.incompat.push(SchemaIncompat::FieldTypeMismatch {
        key: consumer.key().clone(),
        producer: producer_type,
        consumer: consumer.type_name(),
    });
}

/// The three-valued (Cue/GraphQL-style) assignability verdict: a producer
/// schema is provably assignable, provably not, or **not statically decidable**.
///
/// This verdict separates "not provable" ([`Unknown`](Assignability::Unknown)) from "provably wrong"
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
    /// `Dynamic` field, an opaque `Any` producer, or a float→int narrowing).
    /// **Not** fail-open: a strict policy treats this as a blocked edge; a
    /// gradual policy passes it. (Sum-type `Mode` variance is *not* here — it is
    /// decided structurally, yielding `Yes`/`No`.)
    Unknown(Vec<UnknownReason>),
}

/// Why an edge's assignability is [`Unknown`](Assignability::Unknown) — each
/// case is a place where the type system cannot currently *prove* compatibility
/// nor refute it.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnknownReason {
    /// The producer side is opaque, so it cannot be *proven* to match a typed
    /// consumer: either the producer schema is the gradual `Any`
    /// ([`SchemaKind::Any`]), or a matched `List` field's producer item carries
    /// no item schema while the consumer's item is typed (wrapped in a
    /// [`NestedUnknown`](Self::NestedUnknown) under the list key).
    OpaqueProducer,
    /// Scalar domains overlap, but the producer's kind or inclusive bounds do
    /// not prove containment in the consumer domain.
    ScalarNarrowing {
        /// Producer scalar kind.
        producer: ScalarKind,
        /// Consumer scalar kind.
        consumer: ScalarKind,
    },
    /// Consumer root rules are not proven by the producer contract. No rule
    /// contents, predicates, or expression sources enter this diagnostic.
    UnprovenRootRules,
    /// A matched field is `Dynamic`/`Computed` (loader- or expression-backed),
    /// so its concrete shape is unknown until runtime.
    DynamicLoaderBacked {
        /// Key of the dynamic field.
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
    /// A matched pair where at least one side is a [`Field::Unknown`] — a field
    /// kind this version does not recognize. Its value contract is opaque, so
    /// compatibility can be neither proven nor refuted: an older reader cannot
    /// reason about a newer writer's field kind. Routed to `Unknown` (a strict
    /// policy blocks it) rather than a misleading `Yes`/`No`.
    OpaqueFieldKind {
        /// Key of the unrecognized field.
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
                write!(f, "producer side is opaque (shape unknown)")
            },
            Self::ScalarNarrowing { producer, consumer } => {
                write!(
                    f,
                    "scalar domain containment is unproven ({producer:?} to {consumer:?})"
                )
            },
            Self::UnprovenRootRules => write!(f, "consumer root rules are not statically proven"),
            Self::DynamicLoaderBacked { key } => {
                write!(f, "field `{key}` is dynamic/computed (resolved at runtime)")
            },
            Self::NumberWidening { key } => {
                write!(
                    f,
                    "field `{key}` narrows float to integer (possible precision loss)"
                )
            },
            Self::OpaqueFieldKind { key } => {
                write!(
                    f,
                    "field `{key}` is an unrecognized kind (opaque to this version)"
                )
            },
            Self::NestedUnknown { key, inner } => write!(f, "in field `{key}`: {inner}"),
        }
    }
}

/// Polarity-erased ternary assignability core: collects **all** incompatibilities
/// and all "not decidable" reasons rather than stopping at the first. Shared by
/// the public, direction-typed [`explain_assignable`] and
/// [`OutputSchema::explain_successor_of`].
///
/// An `Any` consumer accepts anything ([`Yes`](Assignability::Yes)); an `Any` producer is
/// [`Unknown(OpaqueProducer)`](UnknownReason::OpaqueProducer); two concrete
/// records run strict width-subtyping. Verdict precedence: `No` (any provable
/// incompatibility) ▸ `Unknown` (any undecidable field) ▸ `Yes`.
#[must_use]
pub(crate) fn explain_assignable_core(
    producer: &ValidSchema,
    consumer: &ValidSchema,
) -> Assignability {
    let mut findings = Explain::default();
    match (producer.root_shape(), consumer.root_shape()) {
        (_, RootShape::Any) => return Assignability::Yes,
        (RootShape::Any, _) => {
            return Assignability::Unknown(vec![UnknownReason::OpaqueProducer]);
        },
        (RootShape::Scalar(producer), RootShape::Scalar(consumer)) => {
            collect_scalar(producer, consumer, &mut findings);
        },
        (RootShape::Record(producer), RootShape::Record(consumer)) => {
            collect_fields(producer.fields(), consumer.fields(), true, &mut findings);
        },
        (RootShape::Union(_), RootShape::Union(_)) => {
            return union_assignability(producer, consumer);
        },
        _ => {
            return Assignability::No(vec![SchemaIncompat::KindMismatch {
                producer: producer.kind(),
                consumer: consumer.kind(),
            }]);
        },
    }
    if !consumer.root_rules().iter().all(|rule| {
        matches!(rule.view(), nebula_validator::RuleView::Value(_))
            && producer.root_rules().contains(rule)
    }) {
        findings.unknown.push(UnknownReason::UnprovenRootRules);
    }
    findings.into_verdict()
}

fn collect_scalar(producer: &ScalarSchema, consumer: &ScalarSchema, findings: &mut Explain) {
    let producer_kind = producer.kind();
    let consumer_kind = consumer.kind();
    let numeric_pair = matches!(
        (producer_kind, consumer_kind),
        (
            ScalarKind::Integer | ScalarKind::Number,
            ScalarKind::Integer | ScalarKind::Number
        )
    );
    if producer_kind != consumer_kind && !numeric_pair {
        findings.incompat.push(SchemaIncompat::ScalarKindMismatch {
            producer: producer_kind,
            consumer: consumer_kind,
        });
        return;
    }

    if let (Some(producer_min), Some(producer_max), Some(consumer_min), Some(consumer_max)) = (
        producer.minimum(),
        producer.maximum(),
        consumer.minimum(),
        consumer.maximum(),
    ) {
        if !number_at_least(producer_max, consumer_min)
            || !number_at_least(consumer_max, producer_min)
        {
            findings.incompat.push(SchemaIncompat::ScalarBoundsDisjoint);
        } else if (producer_kind == ScalarKind::Number && consumer_kind == ScalarKind::Integer)
            || !number_at_least(producer_min, consumer_min)
            || !number_at_least(consumer_max, producer_max)
        {
            findings.unknown.push(UnknownReason::ScalarNarrowing {
                producer: producer_kind,
                consumer: consumer_kind,
            });
        }
    }
}

fn number_at_least(value: &Number, minimum: &Number) -> bool {
    // Reuse the validator's exact signed/unsigned/float comparison. Converting
    // either bound to f64 here would collapse adjacent integers above 2^53.
    ValueRule::Min(minimum.clone())
        .validate_value(
            &Value::Number(value.clone()),
            DiagnosticDisclosure::IncludeValue,
        )
        .is_ok()
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
/// Provable conflicts (cardinality, type mismatch, nested, unhandled `Mode`
/// variant) become [`SchemaIncompat`]; the leniencies the binary check silently
/// passes — `Dynamic`/`Computed` and float→int narrowing — become
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

    // `Unknown` on either side: a field kind this version does not recognize.
    // `type_name()` collapses every `Unknown` to the literal `"unknown"`, so the
    // generic `_` arm below would compare two distinct future kinds as equal and
    // emit a misleading `Yes` (or, against a known kind, a hard `No`). The opaque
    // contract is genuinely undecidable here — route it to `Unknown`, symmetric
    // with the `Dynamic`/`Computed` guard above.
    if matches!(producer_field, Field::Unknown(_)) || matches!(consumer_field, Field::Unknown(_)) {
        acc.unknown
            .push(UnknownReason::OpaqueFieldKind { key: key.clone() });
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
                // Producer item untyped but consumer item typed: in the strict
                // (kind-aware) path this is an opaque producer that cannot be
                // *proven* to match the typed item — surface it as Unknown
                // (mirrors the record-level empty-producer rule). The gradual
                // slice path keeps the old producer-side Any escape.
                (None, Some(_)) if strict => {
                    acc.unknown.push(UnknownReason::NestedUnknown {
                        key: key.clone(),
                        inner: Box::new(UnknownReason::OpaqueProducer),
                    });
                },
                // An untyped consumer item accepts any producer item (provably
                // Yes); a gradual untyped producer item also escapes.
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
        (Field::Mode(producer_mode), Field::Mode(consumer_mode)) => {
            collect_mode_variants(key, producer_mode, consumer_mode, strict, acc);
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

/// Sum-type (tagged-union) variance for a matched `Mode` field pair — the dual
/// of record width-subtyping (ADR-0100 §L2 addendum).
///
/// A `Mode` producer may emit *any* of its declared variants at runtime, so the
/// rule is **producer-variant containment**: every producer variant must have a
/// counterpart in the consumer, otherwise the consumer would face a case it
/// cannot handle — a provable [`SchemaIncompat::UnhandledVariant`]. The reverse
/// is sound: a consumer that accepts *more* variants than the producer can emit
/// is still satisfied (the extra arms are simply never taken). For each variant
/// present on both sides the payload is checked **covariantly** — producer
/// payload assignable to consumer payload, the same direction as records — by
/// recursing through [`collect_pair`]; nested payload findings are wrapped under
/// the mode field `key`, while an unhandled variant (which already names both the
/// field and the variant) is reported at the field level.
///
/// Unlike the empty-producer / list-item escapes, variant containment is a real
/// structural check, so it fires in both `strict` and gradual modes; `strict`
/// only governs the per-payload recursion.
fn collect_mode_variants(
    key: &FieldKey,
    producer: &ModeField,
    consumer: &ModeField,
    strict: bool,
    acc: &mut Explain,
) {
    // Push each variant's findings as it is visited (an unhandled variant, or its
    // wrapped payload findings) so the overall order stays depth-first /
    // producer-variant order — the first-incompatibility contract `is_assignable`
    // relies on. (Accumulating all payloads and wrapping once at the end would let
    // a later unhandled variant precede an earlier payload mismatch.)
    for producer_variant in &producer.variants {
        match consumer
            .variants
            .iter()
            .find(|consumer_variant| consumer_variant.key == producer_variant.key)
        {
            None => {
                acc.incompat.push(SchemaIncompat::UnhandledVariant {
                    key: key.clone(),
                    variant: producer_variant.key.clone(),
                });
            },
            Some(consumer_variant) => {
                let mut payload_findings = Explain::default();
                collect_pair(
                    consumer_variant.field.key(),
                    &producer_variant.field,
                    &consumer_variant.field,
                    strict,
                    &mut payload_findings,
                );
                acc.wrap_nested(key, payload_findings);
            },
        }
    }
}

/// Assignability for two checked tagged unions.
///
/// - **Union → Union:** route the two schemas' sole root [`Field::Mode`] (the
///   marker design's variant carrier) through [`collect_mode_variants`] — the
///   *same* producer-variant-containment + covariant-payload rule a nested `Mode`
///   field uses, so there is one declarative sum-type judgment, not two.
fn union_assignability(producer: &ValidSchema, consumer: &ValidSchema) -> Assignability {
    if let (Some(Field::Mode(producer_mode)), Some(Field::Mode(consumer_mode))) =
        (producer.fields().first(), consumer.fields().first())
    {
        // Serde tagging is part of schema identity: two unions with matching
        // variant keys but different tagging (external vs adjacent) have different
        // wire shapes and are NOT interconvertible, so the consumer cannot
        // deserialize the producer's output. Reject before comparing variants.
        if producer.serde_tagging() != consumer.serde_tagging() {
            return Assignability::No(vec![SchemaIncompat::UnionTaggingMismatch {
                producer: producer.serde_tagging().cloned(),
                consumer: consumer.serde_tagging().cloned(),
            }]);
        }
        // Label findings by the consumer's root mode key; both roots are the
        // union's sole field by construction (`ValidSchema::union`).
        let root_key = consumer.fields()[0].key();
        let mut acc = Explain::default();
        collect_mode_variants(root_key, producer_mode, consumer_mode, true, &mut acc);
        return acc.into_verdict();
    }
    // Checked union construction guarantees one mode field. Retain the existing
    // typed rejection if an internal construction regression breaks that invariant.
    Assignability::No(vec![SchemaIncompat::KindMismatch {
        producer: producer.kind(),
        consumer: consumer.kind(),
    }])
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::Field;

    fn fk(s: &str) -> FieldKey {
        FieldKey::new(s).unwrap()
    }

    /// Gradual, kind-blind slice check: an empty producer slice is the `Any`
    /// escape. This is **test-only** — production code calls
    /// [`explain_assignable`], which distinguishes concrete records from Any.
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

    // Keep every schema test on the real direction-typed, tri-state entry point.
    fn explain_assignable(producer: &ValidSchema, consumer: &ValidSchema) -> Assignability {
        super::explain_assignable(
            &OutputSchema::new(producer.clone()),
            &InputSchema::new(consumer.clone()),
        )
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

        let rule = Rule::predicate(Predicate::eq("mode", json!("advanced")).unwrap()).unwrap();
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

    // ── Kind-aware entry point: records and Any stay distinct ──────

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
            explain_assignable(&producer, &consumer),
            Assignability::No(vec![SchemaIncompat::MissingRequiredField {
                key: fk("name")
            }]),
        );
    }

    /// An opaque producer cannot prove the required record contract.
    #[test]
    fn any_producer_is_unknown_for_required_consumer() {
        let producer = ValidSchema::any();
        let consumer = required_record("name");
        assert_eq!(
            explain_assignable(&producer, &consumer),
            Assignability::Unknown(vec![UnknownReason::OpaqueProducer]),
        );
    }

    /// An `Any` consumer accepts any producer (it requires nothing).
    #[test]
    fn typed_producer_satisfies_any_consumer() {
        let producer = required_record("name");
        let consumer = ValidSchema::any();
        assert_eq!(explain_assignable(&producer, &consumer), Assignability::Yes);
    }

    /// An empty record accepts other records under width-subtyping.
    #[test]
    fn empty_record_consumer_accepts_typed_producer() {
        let producer = required_record("name");
        let consumer = ValidSchema::empty();
        assert_eq!(explain_assignable(&producer, &consumer), Assignability::Yes);
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
        assert_eq!(explain_assignable(&producer, &consumer), Assignability::Yes);
    }

    /// Two incompatible concrete records still produce the same first
    /// incompatibility through the kind-aware entry point.
    #[test]
    fn incompatible_records_via_schema_entry_returns_error() {
        let producer = required_record("name");
        let consumer = required_record("other");
        assert_eq!(
            explain_assignable(&producer, &consumer),
            Assignability::No(vec![SchemaIncompat::MissingRequiredField {
                key: fk("other")
            }]),
        );
    }

    /// The universal consumer accepts even an opaque producer.
    #[test]
    fn both_any_schemas_are_compatible() {
        assert_eq!(
            explain_assignable(&ValidSchema::any(), &ValidSchema::any()),
            Assignability::Yes,
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

    /// An `Any` producer is unknown, never proof of a concrete contract.
    #[test]
    fn explain_any_producer_is_unknown_opaque() {
        let consumer = required_record("name");
        assert_eq!(
            explain_assignable(&ValidSchema::any(), &consumer),
            Assignability::Unknown(vec![UnknownReason::OpaqueProducer]),
        );
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

    /// No declared properties does not remove the consumer's object requirement.
    #[test]
    fn explain_any_producer_into_empty_record_consumer_is_unknown() {
        assert_eq!(
            explain_assignable(&ValidSchema::any(), &ValidSchema::empty()),
            Assignability::Unknown(vec![UnknownReason::OpaqueProducer]),
        );
    }

    /// An untyped producer list item against a *typed* consumer item is opaque:
    /// strict ⇒ `Unknown(NestedUnknown { items, OpaqueProducer })`, gradual ⇒
    /// `Yes` (producer-side escape preserved).
    ///
    /// Driven via `explain_slice` on raw `Field`s, not `explain_assignable`: a
    /// built `ValidSchema` can never carry an item-less list (the builder lint
    /// rejects it — `lint.rs` `missing_item_schema`), so this case is
    /// unreachable through the public `ValidSchema` API. The `collect_pair` core
    /// is kept total/correct for it regardless (mirrors the record-level
    /// empty-producer rule), and the gradual slice form does reach it.
    #[test]
    fn explain_untyped_producer_list_item_is_unknown_for_typed_consumer() {
        let p = [Field::list(fk("items")).into()];
        let c = [Field::list(fk("items"))
            .item(Field::string(fk("item")))
            .into()];

        assert_eq!(
            explain_slice(&p, &c, true),
            Assignability::Unknown(vec![UnknownReason::NestedUnknown {
                key: fk("items"),
                inner: Box::new(UnknownReason::OpaqueProducer),
            }]),
        );
        assert_eq!(explain_slice(&p, &c, false), Assignability::Yes);
    }

    /// A dynamic producer has no statically known concrete shape.
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
    }

    /// A `Field::Unknown` on either side is opaque: even two of the *same* future
    /// kind are `Unknown(OpaqueFieldKind)`, never a misleading `Yes` — this
    /// version cannot prove an unrecognized kind's value contract.
    #[test]
    fn explain_unknown_field_pair_is_unknown_not_yes() {
        let producer: ValidSchema =
            serde_json::from_value(json!({"fields": [{"type": "richtext", "key": "bio"}]}))
                .expect("Unknown producer schema");
        let consumer: ValidSchema =
            serde_json::from_value(json!({"fields": [{"type": "richtext", "key": "bio"}]}))
                .expect("Unknown consumer schema");
        assert_eq!(
            explain_assignable(&producer, &consumer),
            Assignability::Unknown(vec![UnknownReason::OpaqueFieldKind { key: fk("bio") }]),
        );
    }

    /// Two *distinct* unknown kinds at the same key are `Unknown`, not a hard
    /// `No`/`FieldTypeMismatch`: `type_name()` collapses both to "unknown", but
    /// the pair is genuinely undecidable, not provably incompatible.
    #[test]
    fn explain_distinct_unknown_kinds_are_unknown_not_mismatch() {
        let producer: ValidSchema =
            serde_json::from_value(json!({"fields": [{"type": "richtext", "key": "bio"}]}))
                .expect("Unknown producer schema");
        let consumer: ValidSchema =
            serde_json::from_value(json!({"fields": [{"type": "gallery", "key": "bio"}]}))
                .expect("Unknown consumer schema");
        assert_eq!(
            explain_assignable(&producer, &consumer),
            Assignability::Unknown(vec![UnknownReason::OpaqueFieldKind { key: fk("bio") }]),
        );
    }

    /// An `Unknown` against a *known* field is also undecidable — not a hard
    /// mismatch — because this version cannot reason about the unknown side.
    #[test]
    fn explain_unknown_vs_known_field_is_unknown_not_mismatch() {
        let producer: ValidSchema =
            serde_json::from_value(json!({"fields": [{"type": "richtext", "key": "bio"}]}))
                .expect("Unknown producer schema");
        let consumer = crate::Schema::builder()
            .add(Field::string(fk("bio")))
            .build()
            .unwrap();
        assert_eq!(
            explain_assignable(&producer, &consumer),
            Assignability::Unknown(vec![UnknownReason::OpaqueFieldKind { key: fk("bio") }]),
        );
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

    /// Identical `Mode`s are provably assignable (`Yes`): every producer variant
    /// has a matching consumer variant with a compatible payload.
    #[test]
    fn explain_mode_identical_is_yes() {
        let mode_schema = || {
            crate::Schema::builder()
                .add(Field::mode(fk("m")).variant("v", "V", Field::string(fk("x"))))
                .build()
                .unwrap()
        };
        let producer = mode_schema();
        let consumer = mode_schema();
        assert_eq!(explain_assignable(&producer, &consumer), Assignability::Yes);
    }

    /// Sum-type containment (the dual of records): a producer `Mode` that can emit
    /// a variant the consumer does not declare is a provable `No` — the consumer
    /// would face an unhandled case. This is the soundness fix: it was previously a
    /// blanket `Unknown` that the binary check let through.
    #[test]
    fn explain_mode_extra_producer_variant_is_unhandled() {
        let producer = crate::Schema::builder()
            .add(
                Field::mode(fk("auth"))
                    .variant("api_key", "API key", Field::string(fk("key")))
                    .variant("oauth", "OAuth", Field::string(fk("token"))),
            )
            .build()
            .unwrap();
        // Consumer handles only `api_key` — it cannot handle the producer's `oauth`.
        let consumer = crate::Schema::builder()
            .add(Field::mode(fk("auth")).variant("api_key", "API key", Field::string(fk("key"))))
            .build()
            .unwrap();
        assert_eq!(
            explain_assignable(&producer, &consumer),
            Assignability::No(vec![SchemaIncompat::UnhandledVariant {
                key: fk("auth"),
                variant: "oauth".to_owned(),
            }]),
        );
    }

    /// The reverse direction is sound: a consumer that accepts *more* variants
    /// than the producer can emit is still satisfied — the extra arms are never
    /// taken.
    #[test]
    fn explain_mode_extra_consumer_variant_is_yes() {
        let producer = crate::Schema::builder()
            .add(Field::mode(fk("auth")).variant("api_key", "API key", Field::string(fk("key"))))
            .build()
            .unwrap();
        let consumer = crate::Schema::builder()
            .add(
                Field::mode(fk("auth"))
                    .variant("api_key", "API key", Field::string(fk("key")))
                    .variant("oauth", "OAuth", Field::string(fk("token"))),
            )
            .build()
            .unwrap();
        assert_eq!(explain_assignable(&producer, &consumer), Assignability::Yes);
    }

    /// Variant payloads are checked covariantly (producer payload → consumer
    /// payload): a shared variant whose payload type mismatches is a nested `No`
    /// keyed by the mode field then the payload field.
    #[test]
    fn explain_mode_variant_payload_mismatch_is_nested() {
        let producer = crate::Schema::builder()
            .add(Field::mode(fk("m")).variant("v", "V", Field::number(fk("x"))))
            .build()
            .unwrap();
        let consumer = crate::Schema::builder()
            .add(Field::mode(fk("m")).variant("v", "V", Field::string(fk("x"))))
            .build()
            .unwrap();
        assert_eq!(
            explain_assignable(&producer, &consumer),
            Assignability::No(vec![SchemaIncompat::NestedIncompat {
                key: fk("m"),
                inner: Box::new(SchemaIncompat::FieldTypeMismatch {
                    key: fk("x"),
                    producer: "number",
                    consumer: "string",
                }),
            }]),
        );
    }

    /// An undecidable payload (a `Dynamic` variant field) bubbles up as a nested
    /// `Unknown`, not a blanket Mode `Unknown` — variance itself is decided.
    #[test]
    fn explain_mode_dynamic_payload_bubbles_unknown() {
        let producer = crate::Schema::builder()
            .add(Field::mode(fk("m")).variant("v", "V", Field::dynamic(fk("x"))))
            .build()
            .unwrap();
        let consumer = crate::Schema::builder()
            .add(Field::mode(fk("m")).variant("v", "V", Field::string(fk("x"))))
            .build()
            .unwrap();
        assert_eq!(
            explain_assignable(&producer, &consumer),
            Assignability::Unknown(vec![UnknownReason::NestedUnknown {
                key: fk("m"),
                inner: Box::new(UnknownReason::DynamicLoaderBacked { key: fk("x") }),
            }]),
        );
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
            "producer side is opaque (shape unknown)"
        );
        assert_eq!(
            UnknownReason::DynamicLoaderBacked { key: fk("tok") }.to_string(),
            "field `tok` is dynamic/computed (resolved at runtime)"
        );
        // Nested reasons render their path prefix.
        assert_eq!(
            UnknownReason::NestedUnknown {
                key: fk("cfg"),
                inner: Box::new(UnknownReason::NumberWidening { key: fk("n") }),
            }
            .to_string(),
            "in field `cfg`: field `n` narrows float to integer (possible precision loss)"
        );
    }

    // ── Directional types: `OutputSchema::explain_successor_of` ─────────

    /// Output-vs-output evolution: a new output that keeps every field old
    /// consumers required (and adds more) is a compatible successor; one that
    /// drops a required field is not. The new output is the producer, the old
    /// output the consumer-expectation.
    #[test]
    fn output_schema_compatible_successor() {
        let prev = OutputSchema::new(required_record("result"));
        let wider = OutputSchema::new(
            crate::Schema::builder()
                .add(Field::string(fk("result")).required())
                .add(Field::string(fk("extra")).required())
                .build()
                .unwrap(),
        );
        assert_eq!(
            wider.explain_successor_of(&prev),
            Assignability::Yes,
            "adding fields keeps old consumers satisfied"
        );

        let narrower = OutputSchema::new(required_record("other"));
        assert_eq!(
            narrower.explain_successor_of(&prev),
            Assignability::No(vec![SchemaIncompat::MissingRequiredField {
                key: fk("result")
            }]),
            "dropping a field old consumers required is a breaking successor"
        );

        // Opaque evolution is unproven, not compatible. Dropping required
        // declarations remains a definite conflict.
        let any_new = OutputSchema::new(ValidSchema::any());
        assert_eq!(
            any_new.explain_successor_of(&prev),
            Assignability::Unknown(vec![UnknownReason::OpaqueProducer]),
            "a new `Any` output is not provably breaking"
        );
        let empty_new = OutputSchema::new(ValidSchema::empty());
        assert_eq!(
            empty_new.explain_successor_of(&prev),
            Assignability::No(vec![SchemaIncompat::MissingRequiredField {
                key: fk("result")
            }]),
            "an empty-record new output drops every field old consumers required"
        );
        // An old Any output imposes no structural constraints.
        let any_prev = OutputSchema::new(ValidSchema::any());
        assert_eq!(
            narrower.explain_successor_of(&any_prev),
            Assignability::Yes,
            "an `Any` old output constrains nothing"
        );
    }

    // ── Sum-type union (SchemaKind::Union) assignability ───────────────────────

    /// An external-tagged union whose variants each carry a string payload.
    fn ext_union(variant_keys: &[&str]) -> ValidSchema {
        let mut mode = Field::mode(fk("u"));
        for key in variant_keys {
            mode = mode.variant(*key, *key, Field::string(fk("x")));
        }
        ValidSchema::union(mode, SerdeTagging::External).expect("union builds")
    }

    /// Identical unions are provably assignable.
    #[test]
    fn union_identical_is_yes() {
        let producer = ext_union(&["a", "b"]);
        let consumer = ext_union(&["a", "b"]);
        assert_eq!(explain_assignable(&producer, &consumer), Assignability::Yes);
    }

    /// Sum-type containment via the SAME `collect_mode_variants` rule: a producer
    /// union that can emit a variant the consumer does not accept is a provable
    /// `No` (`UnhandledVariant`), keyed by the root mode.
    #[test]
    fn union_extra_producer_variant_is_unhandled() {
        let producer = ext_union(&["a", "b"]);
        let consumer = ext_union(&["a"]);
        assert_eq!(
            explain_assignable(&producer, &consumer),
            Assignability::No(vec![SchemaIncompat::UnhandledVariant {
                key: fk("u"),
                variant: "b".to_owned(),
            }]),
        );
    }

    /// The reverse is sound: a consumer union accepting MORE variants than the
    /// producer can emit is satisfied.
    #[test]
    fn union_extra_consumer_variant_is_yes() {
        let producer = ext_union(&["a"]);
        let consumer = ext_union(&["a", "b"]);
        assert_eq!(explain_assignable(&producer, &consumer), Assignability::Yes);
    }

    /// Variant payloads are checked covariantly: a shared variant whose payload
    /// type mismatches is a nested `No` keyed by the root mode then the payload.
    #[test]
    fn union_variant_payload_mismatch_is_nested_no() {
        let producer = ValidSchema::union(
            Field::mode(fk("u")).variant("a", "a", Field::number(fk("x"))),
            SerdeTagging::External,
        )
        .unwrap();
        let consumer = ValidSchema::union(
            Field::mode(fk("u")).variant("a", "a", Field::string(fk("x"))),
            SerdeTagging::External,
        )
        .unwrap();
        assert_eq!(
            explain_assignable(&producer, &consumer),
            Assignability::No(vec![SchemaIncompat::NestedIncompat {
                key: fk("u"),
                inner: Box::new(SchemaIncompat::FieldTypeMismatch {
                    key: fk("x"),
                    producer: "number",
                    consumer: "string",
                }),
            }]),
        );
    }

    /// A union and a plain record are a `KindMismatch` in both directions — a
    /// union value carries a discriminant a record cannot, and vice versa.
    #[test]
    fn union_and_record_are_kind_mismatch() {
        let union = ext_union(&["a"]);
        let record = required_record("name");
        assert_eq!(
            explain_assignable(&union, &record),
            Assignability::No(vec![SchemaIncompat::KindMismatch {
                producer: SchemaKind::Union,
                consumer: SchemaKind::Record,
            }]),
        );
        assert_eq!(
            explain_assignable(&record, &union),
            Assignability::No(vec![SchemaIncompat::KindMismatch {
                producer: SchemaKind::Record,
                consumer: SchemaKind::Union,
            }]),
        );
    }

    /// Gradual escapes still win over the union dispatch (the verified ordering):
    /// an `Any` producer feeding a union consumer is `Unknown` (it *might* emit a
    /// valid tagged value), NOT a hard `KindMismatch`.
    #[test]
    fn any_producer_into_union_consumer_is_unknown() {
        assert_eq!(
            explain_assignable(&ValidSchema::any(), &ext_union(&["a"])),
            Assignability::Unknown(vec![UnknownReason::OpaqueProducer]),
        );
    }

    /// Only Any is universal; an empty record remains a record contract.
    #[test]
    fn union_producer_into_any_is_yes_but_empty_record_is_no() {
        assert_eq!(
            explain_assignable(&ext_union(&["a"]), &ValidSchema::any()),
            Assignability::Yes,
        );
        assert_eq!(
            explain_assignable(&ext_union(&["a"]), &ValidSchema::empty()),
            Assignability::No(vec![SchemaIncompat::KindMismatch {
                producer: SchemaKind::Union,
                consumer: SchemaKind::Record,
            }]),
        );
    }

    /// Two unions with identical variants but DIFFERENT serde tagging are not
    /// assignable — their wire shapes (`{"a": ..}` vs `{tag, content}`) differ and
    /// are not interconvertible, so the consumer cannot deserialize the producer.
    #[test]
    fn union_tagging_mismatch_is_rejected() {
        let external = ext_union(&["a"]); // SerdeTagging::External
        let adjacent = ValidSchema::union(
            Field::mode(fk("u")).variant("a", "a", Field::string(fk("x"))),
            SerdeTagging::Adjacent {
                tag: "t".to_owned(),
                content: "c".to_owned(),
            },
        )
        .unwrap();
        match explain_assignable(&external, &adjacent) {
            Assignability::No(v) => assert!(
                matches!(v.first(), Some(SchemaIncompat::UnionTaggingMismatch { .. })),
                "expected a tagging mismatch, got {v:?}"
            ),
            other => panic!("expected No(UnionTaggingMismatch), got {other:?}"),
        }
    }

    /// Findings keep producer-variant order: an earlier variant's payload mismatch
    /// precedes a later unhandled variant (the first-incompatibility contract).
    #[test]
    fn union_findings_preserve_producer_variant_order() {
        let producer = ValidSchema::union(
            Field::mode(fk("u"))
                .variant("a", "a", Field::number(fk("x")))
                .variant("b", "b", Field::string(fk("y"))),
            SerdeTagging::External,
        )
        .unwrap();
        // Consumer handles only `a`, and types its payload as a string (so `a`
        // mismatches) — `b` is unhandled.
        let consumer = ext_union(&["a"]);
        match explain_assignable(&producer, &consumer) {
            Assignability::No(v) => {
                assert!(
                    matches!(v[0], SchemaIncompat::NestedIncompat { .. }),
                    "earlier variant `a`'s payload mismatch must come first, got {v:?}"
                );
                assert!(
                    matches!(&v[1], SchemaIncompat::UnhandledVariant { variant, .. } if variant == "b"),
                    "later unhandled variant `b` must come second, got {v:?}"
                );
            },
            other => panic!("expected No, got {other:?}"),
        }
    }

    /// Output evolution (sum-type variance): a new output union that ADDS a
    /// variant is a breaking successor (old consumers can't handle it); one that
    /// DROPS a variant is compatible (it just emits fewer cases).
    #[test]
    fn union_output_successor_variance() {
        let old = OutputSchema::new(ext_union(&["a", "b"]));
        let adds = OutputSchema::new(ext_union(&["a", "b", "c"]));
        let drops = OutputSchema::new(ext_union(&["a"]));
        assert_eq!(
            adds.explain_successor_of(&old),
            Assignability::No(vec![SchemaIncompat::UnhandledVariant {
                key: fk("u"),
                variant: "c".to_owned(),
            }]),
            "adding an output variant is breaking — old consumers can't handle it"
        );
        assert_eq!(
            drops.explain_successor_of(&old),
            Assignability::Yes,
            "dropping an output variant is compatible — fewer cases emitted"
        );
    }

    /// A record output cannot evolve into a union output (or vice versa).
    #[test]
    fn record_to_union_successor_is_kind_mismatch() {
        let old = OutputSchema::new(required_record("result"));
        let new = OutputSchema::new(ext_union(&["a"]));
        assert_eq!(
            new.explain_successor_of(&old),
            Assignability::No(vec![SchemaIncompat::KindMismatch {
                producer: SchemaKind::Union,
                consumer: SchemaKind::Record,
            }]),
        );
    }

    // ── explain_field_assignable (field-granularity, W0 U5) ────────────────────

    /// A producer leaf's own key differs from the consumer field it is being
    /// checked against (`email` vs. `recipient`) — `explain_field_assignable`
    /// must re-key both to the same synthetic key internally so the pairing
    /// succeeds, exactly as it would if the two keys already matched.
    #[test]
    fn field_assignable_pairs_leaves_under_different_keys() {
        let producer_leaf: Field = Field::string(fk("email")).required().into();
        let consumer_leaf: Field = Field::string(fk("recipient")).required().into();

        assert_eq!(
            explain_field_assignable(&producer_leaf, &consumer_leaf),
            Assignability::Yes
        );
    }

    #[test]
    fn record_root_is_assignable_to_matching_object_field() {
        let producer = OutputSchema::new(required_record("name"));
        let consumer: Field = Field::object(fk("payload"))
            .add(Field::string(fk("name")).required())
            .into();

        assert_eq!(
            explain_root_field_assignable(&producer, &consumer),
            Assignability::Yes
        );
    }

    #[test]
    fn record_root_is_not_assignable_to_scalar_field() {
        let producer = OutputSchema::new(required_record("name"));
        let consumer: Field = Field::string(fk("payload")).into();

        assert!(matches!(
            explain_root_field_assignable(&producer, &consumer),
            Assignability::No(incompatibilities)
                if matches!(
                    incompatibilities.first(),
                    Some(SchemaIncompat::FieldTypeMismatch {
                        producer: "object",
                        consumer: "string",
                        ..
                    })
                )
        ));
    }

    #[test]
    fn scalar_root_uses_concrete_scalar_kind() {
        let string_root = OutputSchema::new(
            ValidSchema::scalar(ScalarSchema::string()).expect("string root is valid"),
        );
        let string_field: Field = Field::string(fk("payload")).into();
        let number_field: Field = Field::number(fk("payload")).into();

        assert_eq!(
            explain_root_field_assignable(&string_root, &string_field),
            Assignability::Yes
        );
        assert!(matches!(
            explain_root_field_assignable(&string_root, &number_field),
            Assignability::No(_)
        ));
    }

    /// A provable type mismatch between the two leaves is `No`, carrying a
    /// `FieldTypeMismatch` under the consumer's own key.
    #[test]
    fn field_assignable_type_mismatch_is_no() {
        let producer_leaf: Field = Field::number(fk("age")).into();
        let consumer_leaf: Field = Field::string(fk("name")).required().into();

        match explain_field_assignable(&producer_leaf, &consumer_leaf) {
            Assignability::No(incompats) => {
                assert!(incompats.iter().any(|i| matches!(
                    i,
                    SchemaIncompat::FieldTypeMismatch { key, .. } if key.as_str() == "name"
                )));
            },
            other => panic!("expected No(FieldTypeMismatch), got {other:?}"),
        }
    }

    /// A float producer feeding an integer consumer is `Unknown` (possible
    /// precision loss, not a provable conflict) — the same `NumberWidening`
    /// leniency the schema-level check applies.
    #[test]
    fn field_assignable_float_to_int_is_unknown() {
        let producer_leaf: Field = Field::number(fk("amount")).into();
        let consumer_leaf: Field = Field::integer(fk("qty")).required().into();

        assert_eq!(
            explain_field_assignable(&producer_leaf, &consumer_leaf),
            Assignability::Unknown(vec![UnknownReason::NumberWidening { key: fk("qty") }])
        );
    }

    /// A `Field::Unknown` producer leaf at a key DIFFERENT from the required
    /// consumer leaf it is checked against must still report
    /// `Unknown(OpaqueFieldKind)` — not a spurious `No(MissingRequiredField)`.
    ///
    /// `ValidSchema::single_field`'s re-key (`rekeyed`) cannot rewrite an
    /// `Unknown` field's private key, so without the early `Field::Unknown`
    /// check in `explain_field_assignable`, the producer's synthetic
    /// one-field schema would keep the leaf's ORIGINAL key (`"bio"`) instead
    /// of the consumer's key (`"recipient_email"`) — the two synthetic
    /// schemas would fail to pair up, and `collect_fields` would read that as
    /// the required consumer field being entirely absent from the producer.
    #[test]
    fn field_assignable_unknown_producer_leaf_mismatched_key_is_unknown_not_missing() {
        let unknown_schema: ValidSchema =
            serde_json::from_value(json!({"fields": [{"type": "richtext", "key": "bio"}]}))
                .expect("Unknown producer schema");
        let producer_leaf = unknown_schema.fields()[0].clone();
        assert!(
            matches!(producer_leaf, Field::Unknown(_)),
            "sanity: leaf must be Field::Unknown"
        );
        assert_eq!(producer_leaf.key().as_str(), "bio");

        let consumer_leaf: Field = Field::string(fk("recipient_email")).required().into();

        assert_eq!(
            explain_field_assignable(&producer_leaf, &consumer_leaf),
            Assignability::Unknown(vec![UnknownReason::OpaqueFieldKind {
                key: fk("recipient_email")
            }]),
            "an Unknown producer leaf at a different key must report OpaqueFieldKind, not \
             silently fail to pair up as a spurious MissingRequiredField"
        );
    }
}
