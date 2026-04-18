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

use std::sync::OnceLock;

use crate::{option::SelectOption, schema::Schema, validated::ValidSchema, value::FieldValues};

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

/// Types that expose an ordered list of [`SelectOption`] values.
///
/// Typically derived on `enum` types via `#[derive(EnumSelect)]`.
pub trait HasSelectOptions {
    /// Return the options for this type.
    fn select_options() -> Vec<SelectOption>;
}

/// Shared empty schema — cheap `Arc` clone for all baseline impls.
fn empty_schema() -> ValidSchema {
    static EMPTY: OnceLock<ValidSchema> = OnceLock::new();
    EMPTY
        .get_or_init(|| {
            Schema::builder()
                .build()
                .expect("empty schema always builds")
        })
        .clone()
}

/// Baseline `HasSchema` impl for the unit type — actions / credentials /
/// resources that have no user-configurable parameters can use `()` as their
/// `Input` / `Config` without forcing authors to implement the trait.
impl HasSchema for () {
    fn schema() -> ValidSchema {
        empty_schema()
    }
}

/// Baseline `HasSchema` impl for dynamic JSON values — authors using untyped
/// `serde_json::Value` as their `Input` advertise an empty schema. They remain
/// responsible for documenting the expected shape out-of-band. Use a concrete
/// typed struct with `#[derive(Schema)]` to get a real schema.
impl HasSchema for serde_json::Value {
    fn schema() -> ValidSchema {
        empty_schema()
    }
}

/// Baseline `HasSchema` impl for [`FieldValues`] — legacy code paths that
/// still treat the raw value bag as the input type advertise an empty schema.
impl HasSchema for FieldValues {
    fn schema() -> ValidSchema {
        empty_schema()
    }
}

/// Baseline `HasSchema` impls for common primitives — useful when a trait
/// (e.g. [`ResourceConfig`](nebula_resource::ResourceConfig)) requires a
/// `HasSchema` bound and the implementer is using a primitive as a stub.
/// Any production usage should wrap the primitive in a struct with
/// `#[derive(Schema)]` instead.
macro_rules! empty_has_schema_for {
    ($($t:ty),* $(,)?) => {
        $(
            impl HasSchema for $t {
                fn schema() -> ValidSchema {
                    empty_schema()
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
/// schema. In production code, prefer `#[derive(Schema)]` (Phase 2b) so the
/// schema matches the actual struct shape.
#[macro_export]
macro_rules! impl_empty_has_schema {
    ($($t:ty),* $(,)?) => {
        $(
            impl $crate::has_schema::HasSchema for $t {
                fn schema() -> $crate::validated::ValidSchema {
                    $crate::schema::Schema::builder()
                        .build()
                        .expect("empty schema is always valid")
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
    #[allow(
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
    fn unit_has_empty_schema() {
        assert_eq!(<() as HasSchema>::schema().fields().len(), 0);
    }

    #[test]
    fn json_value_has_empty_schema() {
        assert_eq!(<serde_json::Value as HasSchema>::schema().fields().len(), 0);
    }

    #[test]
    fn field_values_has_empty_schema() {
        assert_eq!(<FieldValues as HasSchema>::schema().fields().len(), 0);
    }

    #[test]
    fn empty_schema_is_cached() {
        let a = <() as HasSchema>::schema();
        let b = <serde_json::Value as HasSchema>::schema();
        // Same Arc — shared cache entry.
        assert_eq!(a, b);
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
