//! Schema-compatibility check: structural width-subtyping (TypeDAG T1).
//!
//! The kernel of the ADR-0100 connection type-check. Called by the workflow
//! per-edge validator (T3) to decide whether a producer node's `Output` schema
//! is assignable where a consumer node's `Input` schema is expected.
//!
//! Both [`Schema::fields`](crate::Schema::fields) and
//! [`ValidSchema::fields`](crate::ValidSchema::fields) return `&[Field]`, so
//! callers with either type call `is_assignable(producer.fields(), consumer.fields())`.

use crate::{Field, FieldKey, RequiredMode, SchemaKind, ValidSchema};

// ── Public types ─────────────────────────────────────────────────────────────

/// Why a producer schema is not assignable to a consumer schema.
///
/// Returned by [`is_assignable`] when the structural width-subtyping check
/// fails. Carries the first incompatibility found (depth-first, consumer-field
/// order).
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

/// Structural width-subtyping: are producer fields assignable where consumer
/// fields are expected? (`Output <: Input`, Liskov.)
///
/// Accepts raw field slices so both [`Schema::fields`](crate::Schema::fields)
/// and [`ValidSchema::fields`](crate::ValidSchema::fields) can be passed
/// directly without constructing a wrapper type:
///
/// ```rust
/// use nebula_schema::{Field, Schema, field_key, is_assignable};
///
/// let producer = Schema::builder()
///     .add(Field::string(field_key!("name")).required())
///     .add(Field::number(field_key!("extra")))
///     .build()
///     .unwrap();
///
/// let consumer = Schema::builder()
///     .add(Field::string(field_key!("name")).required())
///     .build()
///     .unwrap();
///
/// assert!(is_assignable(producer.fields(), consumer.fields()).is_ok());
/// ```
///
/// Implements ADR-0100 §L1/L2:
/// - **Width subtyping** — the consumer's required fields must be a subset of
///   the producer's fields with type-compatible matches on the overlap. The
///   producer may emit extra fields; they are ignored.
/// - **`Any` escape (gradual typing)** — an empty slice or a `Dynamic` /
///   `Computed` field on either side is treated as `Any`, so today's
///   `serde_json::Value` (⇒ empty schema) workflows continue to pass. The
///   check only bites when both endpoints carry non-trivial typed schemas.
///   This slice-level entry point is **gradual**: it cannot tell an empty
///   *record* (`()`) from the gradual `Any`, so it treats every empty producer
///   as `Any`. To enforce the Top/Bottom split — an empty record produces no
///   fields and must *not* satisfy a required consumer — use
///   [`is_assignable_schema`], which inspects [`SchemaKind`].
/// - **`Notice` fields** are display-only and are ignored on the consumer side.
/// - Only [`RequiredMode::Always`] consumer fields are hard requirements;
///   [`RequiredMode::When`] and the default optional mode are not enforced
///   statically (the runtime condition cannot be proved at validation time).
/// - **`File` and `Select` cardinality** — the `multiple` flag (scalar vs.
///   array) is checked for equality. A scalar producer paired with an array
///   consumer (or vice versa) is a wire-shape mismatch and returns
///   [`SchemaIncompat::CardinalityMismatch`].
/// - **`Mode` fields** — `Mode`-vs-`Mode` is treated as compatible regardless
///   of variant payloads (lenient, never false-rejects). Real union-variance
///   compatibility (sum-type variance has opposite direction from record
///   width-subtyping) is deferred to an ADR-0100 addendum.
/// - **`Number` integer vs. float** — both carry `type_name() == "number"` and
///   are treated as compatible. Numeric-widening subtyping is deferred to an
///   ADR-0100 addendum.
///
/// # Errors
///
/// Returns the first [`SchemaIncompat`] found (depth-first, consumer-field order):
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
pub fn is_assignable(producer: &[Field], consumer: &[Field]) -> Result<(), SchemaIncompat> {
    // Gradual mode: an empty producer slice is the `Any` escape (see the
    // `strict = false` short-circuit in `fields_assignable`).
    fields_assignable(producer, consumer, false)
}

/// Kind-aware structural assignability: are producer values assignable where
/// consumer values are expected, honoring the [`SchemaKind`] Top/Bottom split?
///
/// Unlike [`is_assignable`], which sees only `&[Field]` and therefore treats
/// *every* empty schema as the gradual `Any`, this entry point distinguishes
/// the gradual `Any` ([`SchemaKind::Any`] — `serde_json::Value`) from an empty
/// *record* ([`SchemaKind::Record`] with no fields — `()`):
///
/// - **Gradual `Any` escape** — if either side is [`SchemaKind::Any`], the
///   producer may emit anything (satisfies any consumer) and the consumer
///   accepts anything, so the check passes. This preserves untyped
///   `serde_json::Value` interop.
/// - **Strict record subtyping** — when **both** sides are concrete records,
///   width-subtyping is enforced *without* the empty-producer `Any` escape: an
///   empty record produces no fields, so it does **not** satisfy a consumer
///   that hard-requires any (it returns [`SchemaIncompat::MissingRequiredField`]).
///   An empty record *consumer* still accepts everything (it requires nothing).
///
/// This is the entry point the workflow per-edge validator and the action
/// output-evolution check use, so the Top/Bottom split is actually enforced on
/// real producer→consumer edges rather than only at the type/serde level.
///
/// # Errors
///
/// Returns the same [`SchemaIncompat`] taxonomy as [`is_assignable`] (the first
/// incompatibility found, depth-first, consumer-field order) when both sides are
/// concrete records and the strict check fails.
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
}
