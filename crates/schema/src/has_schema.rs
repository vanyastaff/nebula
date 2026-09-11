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

use serde_json::Number;

use crate::{
    error::{ValidationError, ValidationReport},
    option::SelectOption,
    validated::{ScalarSchema, ValidSchema},
    value::ValueTree,
};

/// Types that expose a canonical [`ValidSchema`].
///
/// The returned value is cheap to clone — `ValidSchema` is `Arc`-backed.
/// Implementations are expected to be pure: the schema must not depend on
/// runtime state. For dynamic schemas, construct the value explicitly via
/// [`crate::schema::Schema::builder`] and avoid `HasSchema`.
/// Derived implementations cache both successful schemas and construction errors.
pub trait HasSchema {
    /// Return the canonical schema for this type.
    ///
    /// # Errors
    /// Returns the construction or lint report if the type's schema is invalid.
    fn schema() -> Result<ValidSchema, ValidationReport>;
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
/// memoized inside `#[derive(Schema)]`. A caller may still wrap
/// this in its own `OnceLock` if a `&'static` is required.
///
/// # Errors
/// Returns the same construction or lint report as [`HasSchema::schema`].
///
/// # Examples
/// ```
/// use nebula_schema::{SchemaKind, ValidationReport, schema_of};
///
/// let schema = schema_of::<()>()?;
/// assert_eq!(schema.kind(), SchemaKind::Scalar);
/// # Ok::<(), ValidationReport>(())
/// ```
pub fn schema_of<T: HasSchema>() -> Result<ValidSchema, ValidationReport> {
    T::schema()
}

/// Types that expose an ordered list of [`SelectOption`] values.
///
/// Typically derived on `enum` types via `#[derive(EnumSelect)]`.
pub trait HasSelectOptions {
    /// Return the options for this type.
    fn select_options() -> Vec<SelectOption>;
}

/// Baseline `HasSchema` impl for dynamic JSON values — authors using untyped
/// `serde_json::Value` as their `Input` advertise the gradual-typing `Any`: the
/// shape is unknown, not empty. They remain responsible for documenting the
/// expected shape out-of-band. Use a concrete typed struct with
/// `#[derive(Schema)]` to get a real schema.
impl HasSchema for serde_json::Value {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        Ok(ValidSchema::any())
    }
}

/// Dynamic data trees advertise no fixed shape. This does not grant expression
/// admission; an `Any` schema has no expression-permitting declarations.
impl<E> HasSchema for ValueTree<E> {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        Ok(ValidSchema::any())
    }
}

// One cache per concrete primitive, including checked construction failures.
macro_rules! scalar_has_schema_for {
    ($($t:ty => $scalar:expr),* $(,)?) => {
        $(
            impl HasSchema for $t {
                fn schema() -> Result<ValidSchema, ValidationReport> {
                    static SCHEMA: OnceLock<Result<ValidSchema, ValidationReport>> = OnceLock::new();
                    SCHEMA.get_or_init(|| ValidSchema::scalar($scalar)).clone()
                }
            }
        )*
    };
}

scalar_has_schema_for!(
    () => ScalarSchema::null(),
    bool => ScalarSchema::boolean(),
    String => ScalarSchema::string(),
    i8 => ScalarSchema::integer(i8::MIN, i8::MAX)?,
    i16 => ScalarSchema::integer(i16::MIN, i16::MAX)?,
    i32 => ScalarSchema::integer(i32::MIN, i32::MAX)?,
    i64 => ScalarSchema::integer(i64::MIN, i64::MAX)?,
    isize => ScalarSchema::integer(isize::MIN, isize::MAX)?,
    u8 => ScalarSchema::integer(u8::MIN, u8::MAX)?,
    u16 => ScalarSchema::integer(u16::MIN, u16::MAX)?,
    u32 => ScalarSchema::integer(u32::MIN, u32::MAX)?,
    u64 => ScalarSchema::integer(u64::MIN, u64::MAX)?,
    usize => ScalarSchema::integer(usize::MIN, usize::MAX)?,
    // JSON has no native 128-bit number; advertise only the lossless JSON subset.
    i128 => ScalarSchema::integer(i64::MIN, u64::MAX)?,
    u128 => ScalarSchema::integer(0, u64::MAX)?,
    f32 => float_scalar(f64::from(f32::MIN), f64::from(f32::MAX))?,
    f64 => float_scalar(f64::MIN, f64::MAX)?,
);

fn float_scalar(minimum: f64, maximum: f64) -> Result<ScalarSchema, ValidationError> {
    let finite = |number| {
        Number::from_f64(number).ok_or_else(|| {
            ValidationError::builder("schema.scalar_bounds")
                .message("floating-point schema bounds must be finite")
                .build()
        })
    };
    ScalarSchema::number(finite(minimum)?, finite(maximum)?)
}

/// Implement [`HasSchema`] for a genuine empty braced record.
///
/// The type must have an empty-object (`{}`) wire shape. Prefer `#[derive(Schema)]`
/// when possible. This is not a substitute for an undeclared schema: unit types
/// have a null scalar schema, and unknown JSON has [`ValidSchema::any`].
///
/// An empty record producer cannot satisfy a consumer's required fields.
/// An `Any` producer yields `Unknown` against a known consumer, not an affirmative
/// compatibility result. Neither contract should be substituted for the other.
#[macro_export]
macro_rules! impl_empty_has_schema {
    ($($t:ty),* $(,)?) => {
        $(
            impl $crate::has_schema::HasSchema for $t {
                fn schema() -> ::core::result::Result<
                    $crate::validated::ValidSchema,
                    $crate::error::ValidationReport,
                > {
                    ::core::result::Result::Ok($crate::validated::ValidSchema::empty())
                }
            }
        )*
    };
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::AuthoredValue;
    use crate::{field::Field, key::FieldKey, schema::Schema};

    struct Dummy;

    impl HasSchema for Dummy {
        fn schema() -> Result<ValidSchema, ValidationReport> {
            Schema::builder()
                .add(Field::string(FieldKey::new("name")?).required())
                .add(Field::number(FieldKey::new("age")?))
                .build()
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
        let schema = Dummy::schema().unwrap();
        assert_eq!(schema.fields().len(), 2);
        assert_eq!(schema.fields()[0].key().as_str(), "name");
        assert_eq!(schema.fields()[1].key().as_str(), "age");
    }

    #[test]
    fn has_schema_arc_clone_is_cheap() {
        let a = Dummy::schema().unwrap();
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn unit_has_null_scalar_schema() {
        let schema = <() as HasSchema>::schema().unwrap();
        assert_eq!(schema.fields().len(), 0);
        assert_eq!(
            schema.kind(),
            crate::SchemaKind::Scalar,
            "`()` has a null wire value, not an empty record or unknown shape"
        );
        assert_eq!(
            schema.scalar_schema().unwrap().kind(),
            crate::ScalarKind::Null
        );
    }

    #[test]
    fn json_value_has_any_schema() {
        let schema = <serde_json::Value as HasSchema>::schema().unwrap();
        assert_eq!(schema.fields().len(), 0);
        assert_eq!(
            schema.kind(),
            crate::SchemaKind::Any,
            "untyped JSON advertises the gradual `Any`, not an empty record"
        );
    }

    #[test]
    fn field_values_has_any_schema() {
        let schema = <AuthoredValue as HasSchema>::schema().unwrap();
        assert_eq!(schema.fields().len(), 0);
        assert_eq!(schema.kind(), crate::SchemaKind::Any);
    }

    #[test]
    fn primitive_schemas_preserve_known_scalar_kinds() {
        assert_eq!(
            <i32 as HasSchema>::schema()
                .unwrap()
                .scalar_schema()
                .unwrap()
                .kind(),
            crate::ScalarKind::Integer
        );
        assert_eq!(
            <String as HasSchema>::schema()
                .unwrap()
                .scalar_schema()
                .unwrap()
                .kind(),
            crate::ScalarKind::String
        );
    }

    #[test]
    fn unit_and_any_are_distinct_but_each_cached() {
        let unit_a = <() as HasSchema>::schema().unwrap();
        let unit_b = <() as HasSchema>::schema().unwrap();
        let any_a = <serde_json::Value as HasSchema>::schema().unwrap();
        let any_b = <AuthoredValue as HasSchema>::schema().unwrap();

        // Each constructor returns a shared, cached `Arc`.
        assert!(unit_a.ptr_eq(&unit_b), "`()` schema is cached");
        assert!(any_a.ptr_eq(&any_b), "the `Any` schema is shared");

        assert_ne!(
            unit_a, any_a,
            "null must not compare equal to the gradual `Any`"
        );
    }

    #[test]
    fn schema_of_equals_has_schema_schema() {
        // schema_of::<T>() is exactly <T as HasSchema>::schema() — the free
        // helper so call sites need not restate the trait-qualified path.
        assert_eq!(
            schema_of::<Dummy>().unwrap(),
            <Dummy as HasSchema>::schema().unwrap()
        );
        assert_eq!(
            schema_of::<()>().unwrap(),
            <() as HasSchema>::schema().unwrap(),
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
