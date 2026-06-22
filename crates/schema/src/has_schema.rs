//! Traits linking Rust types to schema definitions.
//!
//! A type that implements [`HasSchema`] advertises a canonical [`ValidSchema`]
//! that describes its structure. A type that implements [`HasSelectOptions`]
//! advertises an ordered list of [`SelectOption`] values suitable for a
//! [`SelectField`](crate::field::SelectField).
//!
//! These traits are the bridge between the derive layer (`#[derive(Schema)]`,
//! `#[derive(EnumSelect)]`) and the validator / engine — given a type `T`, a
//! caller can always obtain the schema by name without referring to the derive.

use crate::{option::SelectOption, validated::ValidSchema, value::FieldValues};

/// Types that expose a canonical [`ValidSchema`].
///
/// The returned value is cheap to clone — `ValidSchema` is `Arc`-backed.
/// Implementations are expected to be pure: the schema must not depend on
/// runtime state. For dynamic schemas, construct the value explicitly via
/// [`crate::schema::Schema::builder`] and avoid `HasSchema`.
pub trait HasSchema {
    /// Return the canonical schema for this type.
    fn schema() -> ValidSchema;
}

/// Return the canonical [`ValidSchema`] for `T` without restating the
/// trait-qualified `<T as HasSchema>::schema()` at every call site.
///
/// This is the ergonomic, free-function form of [`HasSchema::schema`] — the
/// single way `Action` / `Credential` / `Resource` consumers reach a
/// companion type's schema. The associated-type bound (e.g. `Action::Input`,
/// `Credential::Properties`, `Resource::Config`) is the sole source of truth;
/// there is no per-trait `*_schema()` method. The returned
/// value is `Arc`-backed and cheap to clone; for derived types it is already
/// memoized inside `#[derive(Schema)]`. a caller may still wrap
/// this in its own `OnceLock` if a `&'static` is required.
#[must_use]
pub fn schema_of<T: HasSchema>() -> ValidSchema {
    T::schema()
}

/// Types that expose an ordered list of [`SelectOption`] values.
///
/// Typically derived on `enum` types via `#[derive(EnumSelect)]`.
pub trait HasSelectOptions {
    /// Return the options for this type.
    fn select_options() -> Vec<SelectOption>;
}

/// Shared empty-record schema — delegates to [`ValidSchema::empty`].
///
/// This is a [`SchemaKind::Record`](crate::SchemaKind::Record) with zero fields:
/// the type genuinely has no inputs. Distinct from [`any_schema`], which is the
/// gradual-typing `Any` for types whose shape is unknown.
fn empty_schema() -> ValidSchema {
    ValidSchema::empty()
}

/// Shared gradual-typing `Any` schema — delegates to [`ValidSchema::any`].
///
/// Used for types that carry data of an unknown record shape (`serde_json::Value`,
/// [`FieldValues`], primitive stubs). Unlike [`empty_schema`] it is *not* an
/// empty record — it is the lattice `Any`, so it stays distinct from `()` and
/// does not falsely claim to produce or consume zero fields.
fn any_schema() -> ValidSchema {
    ValidSchema::any()
}

/// Baseline `HasSchema` impl for the unit type — actions / credentials /
/// resources that have no user-configurable parameters can use `()` as their
/// `Input` / `Config` without forcing authors to implement the trait.
///
/// `()` is an empty *record* (`SchemaKind::Record`): it genuinely has no inputs,
/// as opposed to the untyped `Any` of `serde_json::Value`.
impl HasSchema for () {
    fn schema() -> ValidSchema {
        empty_schema()
    }
}

/// Baseline `HasSchema` impl for dynamic JSON values — authors using untyped
/// `serde_json::Value` as their `Input` advertise the gradual-typing `Any`: the
/// shape is unknown, not empty. They remain responsible for documenting the
/// expected shape out-of-band. Use a concrete typed struct with
/// `#[derive(Schema)]` to get a real schema.
impl HasSchema for serde_json::Value {
    fn schema() -> ValidSchema {
        any_schema()
    }
}

/// Baseline `HasSchema` impl for [`FieldValues`] — legacy code paths that still
/// treat the raw value bag as the input type advertise the gradual-typing `Any`
/// (unknown shape), not an empty record.
impl HasSchema for FieldValues {
    fn schema() -> ValidSchema {
        any_schema()
    }
}

/// Baseline `HasSchema` impls for common primitives — useful when a trait
/// (e.g. [`ResourceConfig`](nebula_resource::ResourceConfig)) requires a
/// `HasSchema` bound and the implementer is using a primitive as a stub.
/// A bare scalar carries data of no record shape, so it advertises the
/// gradual-typing `Any` rather than an empty record. Any production usage
/// should wrap the primitive in a struct with `#[derive(Schema)]` instead.
macro_rules! empty_has_schema_for {
    ($($t:ty),* $(,)?) => {
        $(
            impl HasSchema for $t {
                fn schema() -> ValidSchema {
                    any_schema()
                }
            }
        )*
    };
}

empty_has_schema_for!(
    bool, String, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64
);

/// Convenience macro: `nebula_schema::impl_empty_has_schema!(MyStub);`
///
/// Emits a [`HasSchema`] implementation that returns an empty [`ValidSchema`].
/// Suitable for test fixtures / legacy types that don't yet declare a real
/// schema. In production code, prefer `#[derive(Schema)]` so the
/// schema matches the actual struct shape.
///
/// The emitted schema is an empty **record** (`SchemaKind::Record`), not the
/// gradual `Any`. Used as an action `Output`, it is the genuine "no fields"
/// case: under [`is_assignable_schema`](crate::is_assignable_schema) it will
/// **fail** the strict check against a consumer that hard-requires a field. If
/// gradual typing (an `Any` that satisfies any consumer) is what you want, do
/// not declare an empty record — use an untyped `serde_json::Value` input, or
/// implement [`HasSchema`] returning [`ValidSchema::any`](crate::ValidSchema::any).
#[macro_export]
macro_rules! impl_empty_has_schema {
    ($($t:ty),* $(,)?) => {
        $(
            impl $crate::has_schema::HasSchema for $t {
                fn schema() -> $crate::validated::ValidSchema {
                    $crate::validated::ValidSchema::empty()
                }
            }
        )*
    };
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{field::Field, key::FieldKey, schema::Schema};

    struct Dummy;

    impl HasSchema for Dummy {
        fn schema() -> ValidSchema {
            Schema::builder()
                .add(Field::string(FieldKey::new("name").unwrap()).required())
                .add(Field::number(FieldKey::new("age").unwrap()))
                .build()
                .expect("dummy schema builds")
        }
    }

    #[derive(Clone, Copy)]
    #[expect(
        dead_code,
        reason = "variants exercised via HasSelectOptions impl only"
    )]
    enum Color {
        Red,
        Green,
        Blue,
    }

    impl HasSelectOptions for Color {
        fn select_options() -> Vec<SelectOption> {
            vec![
                SelectOption::new(json!("red"), "Red"),
                SelectOption::new(json!("green"), "Green"),
                SelectOption::new(json!("blue"), "Blue"),
            ]
        }
    }

    #[test]
    fn has_schema_returns_valid_schema() {
        let schema = Dummy::schema();
        assert_eq!(schema.fields().len(), 2);
        assert_eq!(schema.fields()[0].key().as_str(), "name");
        assert_eq!(schema.fields()[1].key().as_str(), "age");
    }

    #[test]
    fn has_schema_arc_clone_is_cheap() {
        let a = Dummy::schema();
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn unit_has_empty_record_schema() {
        let schema = <() as HasSchema>::schema();
        assert_eq!(schema.fields().len(), 0);
        assert_eq!(
            schema.kind(),
            crate::SchemaKind::Record,
            "`()` is a genuine empty record, not the gradual `Any`"
        );
    }

    #[test]
    fn json_value_has_any_schema() {
        let schema = <serde_json::Value as HasSchema>::schema();
        assert_eq!(schema.fields().len(), 0);
        assert_eq!(
            schema.kind(),
            crate::SchemaKind::Any,
            "untyped JSON advertises the gradual `Any`, not an empty record"
        );
    }

    #[test]
    fn field_values_has_any_schema() {
        let schema = <FieldValues as HasSchema>::schema();
        assert_eq!(schema.fields().len(), 0);
        assert_eq!(schema.kind(), crate::SchemaKind::Any);
    }

    #[test]
    fn primitive_stub_has_any_schema() {
        assert_eq!(<i32 as HasSchema>::schema().kind(), crate::SchemaKind::Any);
        assert_eq!(
            <String as HasSchema>::schema().kind(),
            crate::SchemaKind::Any
        );
    }

    #[test]
    fn unit_and_any_are_distinct_but_each_cached() {
        let unit_a = <() as HasSchema>::schema();
        let unit_b = <() as HasSchema>::schema();
        let any_a = <serde_json::Value as HasSchema>::schema();
        let any_b = <FieldValues as HasSchema>::schema();

        // Each constructor returns a shared, cached `Arc`.
        assert!(unit_a.ptr_eq(&unit_b), "`()` schema is cached");
        assert!(any_a.ptr_eq(&any_b), "the `Any` schema is shared");

        // But the empty record and the gradual `Any` are NOT the same schema:
        // distinguishing them is the whole point of the Top/Bottom split.
        assert_ne!(
            unit_a, any_a,
            "an empty record must not compare equal to the gradual `Any`"
        );
    }

    #[test]
    fn schema_of_equals_has_schema_schema() {
        // schema_of::<T>() is exactly <T as HasSchema>::schema() — the free
        // helper so call sites need not restate the trait-qualified path.
        assert_eq!(schema_of::<Dummy>(), <Dummy as HasSchema>::schema());
        assert_eq!(
            schema_of::<()>(),
            <() as HasSchema>::schema(),
            "unit blanket impl routes through schema_of"
        );
    }

    #[test]
    fn has_select_options_returns_ordered_list() {
        let options = Color::select_options();
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].label, "Red");
        assert_eq!(options[1].value, json!("green"));
        assert_eq!(options[2].label, "Blue");
    }
}
