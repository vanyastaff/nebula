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
    FieldKey, InputSchema, OutputSchema, Property, RequiredMode, RootShape, ScalarKind,
    ScalarSchema, SchemaKind, SerdeTagging, ValidSchema, field::ModeField,
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
    /// A field present on both sides has incompatible types (different `Property`
    /// variants). The `producer` and `consumer` strings are the
    /// [`Property::type_name`] values — `"string"`, `"number"`, etc.
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
/// # Property Contracts
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
///     Assignability, Property, InputSchema, OutputSchema, Schema, UnknownReason,
///     ValidSchema, explain_assignable, field_key,
/// };
///
/// let consumer = InputSchema::new(
///     Schema::builder()
///         .property(Property::string(field_key!("name")).required())
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

/// Property-granularity counterpart to [`explain_assignable`]: is `producer_leaf`
/// assignable where `consumer_leaf` is expected, when a caller has two
/// individual resolved [`Property`]s in hand rather than two whole schemas (e.g.
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
/// A [`Property::Unknown`] leaf is handled BEFORE that synthetic-schema
/// machinery runs, not after: `ValidSchema::single_field`'s re-keying
/// (`rekeyed`) cannot rewrite an `Unknown` field's private key (see its own
/// doc comment), so a `Property::Unknown` producer or consumer leaf whose
/// original key differs from the other side's key would fail to pair up
/// under the shared synthetic key — surfacing as a spurious
/// `No(MissingRequiredField)` (required consumer field) or a spurious `Yes`
/// (optional consumer field), rather than the correct "this version cannot
/// reason about an unrecognized field kind" verdict.
#[must_use]
pub fn explain_field_assignable(
    producer_leaf: &Property,
    consumer_leaf: &Property,
) -> Assignability {
    // `Property::Unknown` is opaque on either side (mirrors `collect_pair`'s own
    // same-key `Property::Unknown` handling a few lines below) — decide this
    // before building any synthetic schema, since the re-key that machinery
    // relies on cannot reach an `Unknown` field's key.
    if matches!(producer_leaf, Property::Unknown(_))
        || matches!(consumer_leaf, Property::Unknown(_))
    {
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
    consumer_field: &Property,
) -> Assignability {
    let producer_schema = producer_root.as_schema();
    if !producer_schema.has_current_policy() {
        return Assignability::Unknown(vec![UnknownReason::UnsupportedPolicy]);
    }
    let mut findings = Explain::default();
    match producer_schema.root_shape() {
        RootShape::Any | RootShape::Union(_) => {
            return Assignability::Unknown(vec![UnknownReason::OpaqueProducer]);
        },
        RootShape::Record(producer_record) => match consumer_field {
            Property::Object(consumer_object) => collect_fields(
                producer_record.properties(),
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

fn collect_root_scalar_field(producer: &ScalarSchema, consumer: &Property, findings: &mut Explain) {
    let is_compatible = matches!(
        (producer.kind(), consumer),
        (ScalarKind::String, Property::String(_))
            | (ScalarKind::Boolean, Property::Boolean(_))
            | (ScalarKind::Integer, Property::Number(_))
            | (
                ScalarKind::Number,
                Property::Number(crate::NumberField { integer: false, .. })
            )
    );
    if is_compatible {
        return;
    }
    if matches!(
        (producer.kind(), consumer),
        (
            ScalarKind::Number,
            Property::Number(crate::NumberField { integer: true, .. })
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
    /// Historical policy evidence cannot prove a current data contract.
    UnsupportedPolicy,
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
    /// A matched pair where at least one side is a [`Property::Unknown`] — a field
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
            Self::UnsupportedPolicy => f.write_str("historical schema policy is unsupported"),
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
    if !producer.has_current_policy() || !consumer.has_current_policy() {
        return Assignability::Unknown(vec![UnknownReason::UnsupportedPolicy]);
    }
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
            collect_fields(
                producer.properties(),
                consumer.properties(),
                true,
                &mut findings,
            );
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
    producer_fields: &[Property],
    consumer_fields: &[Property],
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
        if matches!(consumer_field, Property::Notice(_)) {
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
    producer_field: &Property,
    consumer_field: &Property,
    strict: bool,
    acc: &mut Explain,
) {
    // Dynamic/Computed on either side: loader/expression-backed, concrete shape
    // unknown until runtime — not statically provable in either direction.
    if matches!(producer_field, Property::Dynamic(_) | Property::Computed(_))
        || matches!(consumer_field, Property::Dynamic(_) | Property::Computed(_))
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
    if matches!(producer_field, Property::Unknown(_))
        || matches!(consumer_field, Property::Unknown(_))
    {
        acc.unknown
            .push(UnknownReason::OpaqueFieldKind { key: key.clone() });
        return;
    }

    match (producer_field, consumer_field) {
        (Property::File(p), Property::File(c)) => {
            if p.multiple != c.multiple {
                acc.incompat.push(SchemaIncompat::CardinalityMismatch {
                    key: key.clone(),
                    producer_multiple: p.multiple,
                    consumer_multiple: c.multiple,
                });
            }
        },
        (Property::Select(p), Property::Select(c)) => {
            if p.multiple != c.multiple {
                acc.incompat.push(SchemaIncompat::CardinalityMismatch {
                    key: key.clone(),
                    producer_multiple: p.multiple,
                    consumer_multiple: c.multiple,
                });
            }
        },
        (Property::Object(producer_obj), Property::Object(consumer_obj)) => {
            let mut sub = Explain::default();
            collect_fields(&producer_obj.fields, &consumer_obj.fields, strict, &mut sub);
            acc.wrap_nested(key, sub);
        },
        (Property::List(producer_list), Property::List(consumer_list)) => {
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
        (Property::Number(p), Property::Number(c)) => {
            // int→float widening is safe (provably Yes). float→int narrowing may
            // lose precision — but a float producer could still only ever emit
            // integral values, so it is not provably wrong: Unknown, not No.
            if !p.integer && c.integer {
                acc.unknown
                    .push(UnknownReason::NumberWidening { key: key.clone() });
            }
        },
        (Property::Mode(producer_mode), Property::Mode(consumer_mode)) => {
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
/// - **Union → Union:** route the two schemas' sole root [`Property::Mode`] (the
///   marker design's variant carrier) through [`collect_mode_variants`] — the
///   *same* producer-variant-containment + covariant-payload rule a nested `Mode`
///   field uses, so there is one declarative sum-type judgment, not two.
fn union_assignability(producer: &ValidSchema, consumer: &ValidSchema) -> Assignability {
    if let (Some(Property::Mode(producer_mode)), Some(Property::Mode(consumer_mode))) =
        (producer.properties().first(), consumer.properties().first())
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
        let root_key = consumer.properties()[0].key();
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
mod tests;
