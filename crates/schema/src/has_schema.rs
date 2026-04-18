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

use crate::{option::SelectOption, validated::ValidSchema};

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
    fn has_select_options_returns_ordered_list() {
        let options = Color::select_options();
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].label, "Red");
        assert_eq!(options[1].value, json!("green"));
        assert_eq!(options[2].label, "Blue");
    }
}
