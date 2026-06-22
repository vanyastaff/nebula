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
/// - **`Mode` fields** — `Mode`-vs-`Mode` is treated as compatible regardless
///   of variant payloads (lenient, never false-rejects). Real union-variance
///   compatibility is deferred to an ADR-0100 addendum.
/// - **`Number` integer vs. float** — both carry `type_name() == "number"` and
///   are treated as compatible. Numeric-widening subtyping is deferred to an
///   ADR-0100 addendum.
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
    // Gradual `Any` on either side is the escape hatch.
    if producer.kind() == SchemaKind::Any || consumer.kind() == SchemaKind::Any {
        return Ok(());
    }
    // Both are concrete records: strict width-subtyping, no empty-producer escape.
    fields_assignable(producer.fields(), consumer.fields(), true)
}

// ── Private core ─────────────────────────────────────────────────────────────

/// Core field-slice assignability loop, shared between the top-level entry
/// point and recursive `Object`/`List` descent.
///
/// Keeping this as a separate private function avoids constructing throwaway
/// `Schema` values for nested object/list fields during recursion.
fn fields_assignable(
    producer_fields: &[Field],
    consumer_fields: &[Field],
    strict: bool,
) -> Result<(), SchemaIncompat> {
    // An empty consumer requires nothing — always satisfiable, both modes.
    if consumer_fields.is_empty() {
        return Ok(());
    }
    // Gradual mode: an empty producer is the untyped/opaque `Any` escape. In
    // strict mode the producer is a concrete record that emits exactly these
    // fields, so an empty producer must face the per-field required check below
    // (and will fail any hard-required consumer field).
    if !strict && producer_fields.is_empty() {
        return Ok(());
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
                return Err(SchemaIncompat::MissingRequiredField {
                    key: consumer_key.clone(),
                });
            },
            None => {
                // Optional consumer field absent from producer — fine under
                // width subtyping.
                continue;
            },
            Some(producer_field) => {
                field_pair_assignable(consumer_key, producer_field, consumer_field, strict)?;
            },
        }
    }

    Ok(())
}

/// Check a matched field pair (same key, both present).
///
/// - `Dynamic`/`Computed` on either side → Any escape → `Ok`.
/// - `File`/`File` and `Select`/`Select` → check `multiple` equality →
///   [`SchemaIncompat::CardinalityMismatch`] if they differ.
/// - Same structural variant with nested fields → recurse; wrap inner error in
///   [`SchemaIncompat::NestedIncompat`].
/// - Different structural variants (different `type_name`) →
///   [`SchemaIncompat::FieldTypeMismatch`].
/// - Same primitive variant → `Ok`.
///
/// ## Mode fields
///
/// `Mode`-vs-`Mode` falls through to `type_name()` equality → always `Ok`
/// regardless of variant payloads.
/// NOTE: `Mode` is a sum type; sum-type variance (contravariant on the
/// argument side) is the opposite of record width-subtyping and is deferred to
/// an ADR-0100 addendum. This arm is intentionally lenient — it never
/// false-rejects a `Mode` pair.
fn field_pair_assignable(
    key: &FieldKey,
    producer_field: &Field,
    consumer_field: &Field,
    strict: bool,
) -> Result<(), SchemaIncompat> {
    // Any escape: Dynamic or Computed on either side matches anything. This
    // per-field gradual escape holds in both modes — a `Dynamic`/`Computed`
    // field is explicitly `Any`-typed regardless of the enclosing record's kind.
    if matches!(producer_field, Field::Dynamic(_) | Field::Computed(_))
        || matches!(consumer_field, Field::Dynamic(_) | Field::Computed(_))
    {
        return Ok(());
    }

    match (producer_field, consumer_field) {
        (Field::File(p), Field::File(c)) => {
            if p.multiple != c.multiple {
                return Err(SchemaIncompat::CardinalityMismatch {
                    key: key.clone(),
                    producer_multiple: p.multiple,
                    consumer_multiple: c.multiple,
                });
            }
            Ok(())
        },
        (Field::Select(p), Field::Select(c)) => {
            if p.multiple != c.multiple {
                return Err(SchemaIncompat::CardinalityMismatch {
                    key: key.clone(),
                    producer_multiple: p.multiple,
                    consumer_multiple: c.multiple,
                });
            }
            Ok(())
        },
        (Field::Object(producer_obj), Field::Object(consumer_obj)) => {
            fields_assignable(&producer_obj.fields, &consumer_obj.fields, strict).map_err(|inner| {
                SchemaIncompat::NestedIncompat {
                    key: key.clone(),
                    inner: Box::new(inner),
                }
            })
        },
        (Field::List(producer_list), Field::List(consumer_list)) => {
            match (&producer_list.item, &consumer_list.item) {
                // Either side has no typed item schema — Any escape.
                (None, _) | (_, None) => Ok(()),
                (Some(producer_item), Some(consumer_item)) => {
                    field_pair_assignable(key, producer_item, consumer_item, strict).map_err(
                        |inner| SchemaIncompat::NestedIncompat {
                            key: key.clone(),
                            inner: Box::new(inner),
                        },
                    )
                },
            }
        },
        // For all other variant pairs: same type_name = compatible.
        // Note: Mode-vs-Mode is intentionally lenient — see fn-level NOTE above.
        // Note: Number integer-vs-float both have type_name "number" — deferred to ADR-0100 addendum.
        _ => {
            if producer_field.type_name() == consumer_field.type_name() {
                Ok(())
            } else {
                Err(SchemaIncompat::FieldTypeMismatch {
                    key: key.clone(),
                    producer: producer_field.type_name(),
                    consumer: consumer_field.type_name(),
                })
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
    /// directly on `&[Field]` without building a `ValidSchema` per case.
    fn is_assignable(producer: &[Field], consumer: &[Field]) -> Result<(), SchemaIncompat> {
        fields_assignable(producer, consumer, false)
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
        assert_eq!(
            is_assignable(&producer, &consumer),
            Err(SchemaIncompat::NestedIncompat {
                key: fk("values"),
                inner: Box::new(SchemaIncompat::FieldTypeMismatch {
                    key: fk("values"),
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
    // These drive the private core directly (the public `is_assignable_schema`
    // delegates record subtyping to `fields_assignable(.., strict = true)`), and
    // contrast strict vs gradual at depth so a dropped `strict` argument in the
    // Object/List recursion arms goes RED.

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
            fields_assignable(&producer, &consumer, true),
            Err(SchemaIncompat::NestedIncompat {
                key: fk("config"),
                inner: Box::new(SchemaIncompat::MissingRequiredField { key: fk("host") }),
            }),
            "strict mode must recurse into the empty producer object and fail the required field"
        );
        assert_eq!(
            fields_assignable(&producer, &consumer, false),
            Ok(()),
            "gradual mode escapes the empty nested producer — confirms the strict flag is what bites"
        );
    }

    /// A list whose item is an empty object does NOT satisfy a consumer list
    /// whose item object hard-requires a field — strict propagates List → item →
    /// Object. Gradual mode escapes it.
    #[test]
    fn strict_rejects_empty_nested_list_item_producer() {
        let producer = [Field::list(fk("items"))
            .item(Field::object(fk("item")))
            .into()];
        let consumer = [Field::list(fk("items"))
            .item(Field::object(fk("item")).add(Field::string(fk("id")).required()))
            .into()];

        assert_eq!(
            fields_assignable(&producer, &consumer, true),
            Err(SchemaIncompat::NestedIncompat {
                key: fk("items"),
                inner: Box::new(SchemaIncompat::NestedIncompat {
                    key: fk("items"),
                    inner: Box::new(SchemaIncompat::MissingRequiredField { key: fk("id") }),
                }),
            }),
            "strict mode must recurse List -> item -> Object and fail the required field"
        );
        assert_eq!(
            fields_assignable(&producer, &consumer, false),
            Ok(()),
            "gradual mode escapes the empty list-item object — confirms strict propagation"
        );
    }
}
