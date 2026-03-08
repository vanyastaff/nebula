//! Macros for defining subtypes in a declarative way.
//!
//! Inspired by paramdef's `define_*_subtype!` macros.

/// Defines a text subtype with compile-time metadata.
///
/// # Syntax
///
/// ```ignore
/// define_text_subtype!(
///     TypeName,
///     name: "string_name",
///     description: "Human description",
///     [optional fields...]
/// );
/// ```
///
/// # Optional Fields
///
/// - `pattern: "regex"` - Validation regex
/// - `sensitive: true` - Mark as sensitive data
/// - `code: true` - Mark as code content
/// - `multiline: true` - Enable multiline input
/// - `placeholder: "text"` - Placeholder for UI
///
/// # Examples
///
/// ```ignore
/// use nebula_parameter::define_text_subtype;
///
/// define_text_subtype!(
///     EmailAddr,
///     name: "email",
///     description: "Email address",
///     pattern: r"^[^@]+@[^@]+\.[^@]+$",
///     placeholder: "user@example.com"
/// );
///
/// define_text_subtype!(
///     SecretToken,
///     name: "token",
///     description: "Secret token",
///     sensitive: true
/// );
/// ```
#[macro_export]
macro_rules! define_text_subtype {
    (
        $name:ident,
        name: $str_name:literal,
        description: $desc:literal
        $(, pattern: $pattern:literal)?
        $(, sensitive: $sensitive:literal)?
        $(, code: $code:literal)?
        $(, multiline: $multiline:literal)?
        $(, placeholder: $placeholder:literal)?
    ) => {
        /// Text subtype.
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
        pub struct $name;

        impl serde::Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str($str_name)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = <String as serde::Deserialize>::deserialize(deserializer)?;
                if value == $str_name {
                    Ok($name)
                } else {
                    Err(serde::de::Error::custom(format!(
                        "expected '{}', got '{}'",
                        $str_name, value
                    )))
                }
            }
        }

        impl $crate::subtype::traits::TextSubtype for $name {
            #[inline]
            fn name() -> &'static str {
                $str_name
            }

            #[inline]
            fn description() -> &'static str {
                $desc
            }

            $(
                #[inline]
                fn pattern() -> Option<&'static str> {
                    Some($pattern)
                }
            )?

            $(
                #[inline]
                fn is_sensitive() -> bool {
                    $sensitive
                }
            )?

            $(
                #[inline]
                fn is_code() -> bool {
                    $code
                }
            )?

            $(
                #[inline]
                fn is_multiline() -> bool {
                    $multiline
                }
            )?

            $(
                #[inline]
                fn placeholder() -> Option<&'static str> {
                    Some($placeholder)
                }
            )?
        }
    };
}

/// Defines a number subtype with compile-time type constraints.
///
/// # Syntax
///
/// ```ignore
/// define_number_subtype!(
///     TypeName,
///     ValueType,  // i64 or f64
///     name: "string_name",
///     description: "Human description",
///     [optional fields...]
/// );
/// ```
///
/// # Optional Fields
///
/// - `range: (min, max)` - Default range constraints
/// - `step: value` - Default step for UI
/// - `percentage: true` - Mark as percentage type
///
/// # Examples
///
/// ```ignore
/// use nebula_parameter::define_number_subtype;
///
/// // Integer-only subtype
/// define_number_subtype!(
///     PortNumber,
///     i64,
///     name: "port",
///     description: "Network port",
///     range: (1, 65535)
/// );
///
/// // Float subtype
/// define_number_subtype!(
///     Factor,
///     f64,
///     name: "factor",
///     description: "Multiplicative factor",
///     range: (0.0, 1.0)
/// );
/// ```
#[macro_export]
macro_rules! define_number_subtype {
    (
        $name:ident,
        $value_type:ty,
        name: $str_name:literal,
        description: $desc:literal
        $(, range: ($min:expr, $max:expr))?
        $(, step: $step:expr)?
        $(, percentage: $percentage:literal)?
    ) => {
        /// Number subtype.
        #[derive(Debug, Clone, Copy, Default, PartialEq)]
        pub struct $name;

        impl serde::Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str($str_name)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = <String as serde::Deserialize>::deserialize(deserializer)?;
                if value == $str_name {
                    Ok($name)
                } else {
                    Err(serde::de::Error::custom(format!(
                        "expected '{}', got '{}'",
                        $str_name, value
                    )))
                }
            }
        }

        impl $crate::subtype::traits::NumberSubtype for $name {
            type Value = $value_type;

            #[inline]
            fn name() -> &'static str {
                $str_name
            }

            #[inline]
            fn description() -> &'static str {
                $desc
            }

            $(
                #[inline]
                fn default_range() -> Option<(Self::Value, Self::Value)> {
                    Some(($min, $max))
                }
            )?

            $(
                #[inline]
                fn default_step() -> Option<Self::Value> {
                    Some($step)
                }
            )?

            $(
                #[inline]
                fn is_percentage() -> bool {
                    $percentage
                }
            )?
        }

        // Compile-time check: Value type must implement Numeric
        const _: () = {
            fn _assert_numeric<T: $crate::subtype::traits::Numeric>() {}
            fn _check() {
                _assert_numeric::<$value_type>();
            }
        };
    };
}

#[cfg(test)]
mod tests {
    use crate::subtype::traits::{NumberSubtype, TextSubtype};

    define_text_subtype!(
        TestEmail,
        name: "test_email",
        description: "Test email address",
        pattern: r"^.+@.+$",
        placeholder: "test@example.com"
    );

    define_text_subtype!(
        TestSecret,
        name: "test_secret",
        description: "Test secret",
        sensitive: true
    );

    define_number_subtype!(
        TestPort,
        i64,
        name: "test_port",
        description: "Test port number",
        range: (1, 65535)
    );

    define_number_subtype!(
        TestPercentage,
        f64,
        name: "test_percentage",
        description: "Test percentage",
        range: (0.0, 100.0),
        percentage: true
    );

    #[test]
    fn test_text_subtype_macro() {
        assert_eq!(TestEmail::name(), "test_email");
        assert_eq!(TestEmail::description(), "Test email address");
        assert_eq!(TestEmail::pattern(), Some(r"^.+@.+$"));
        assert_eq!(TestEmail::placeholder(), Some("test@example.com"));
        assert!(!TestEmail::is_sensitive());
    }

    #[test]
    fn test_sensitive_text_subtype() {
        assert_eq!(TestSecret::name(), "test_secret");
        assert!(TestSecret::is_sensitive());
    }

    #[test]
    fn test_number_subtype_macro() {
        assert_eq!(TestPort::name(), "test_port");
        assert_eq!(TestPort::description(), "Test port number");
        assert_eq!(TestPort::default_range(), Some((1, 65535)));
    }

    #[test]
    fn test_percentage_subtype() {
        assert_eq!(TestPercentage::name(), "test_percentage");
        assert!(TestPercentage::is_percentage());
        assert_eq!(TestPercentage::default_range(), Some((0.0, 100.0)));
    }
}
