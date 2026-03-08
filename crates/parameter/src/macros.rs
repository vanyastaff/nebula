//! Convenient macros for working with parameters.

/// Create [`ParameterValues`](crate::values::ParameterValues) from key-value pairs.
///
/// # Examples
///
/// ```
/// use nebula_parameter::param_values;
/// use serde_json::json;
///
/// let values = param_values! {
///     "api_key" => "secret123",
///     "timeout" => 30,
///     "enabled" => true,
///     "config" => json!({"host": "localhost", "port": 8080}),
/// };
///
/// assert_eq!(values.get_string("api_key"), Some("secret123"));
/// assert_eq!(values.get_f64("timeout"), Some(30.0));
/// assert_eq!(values.get_bool("enabled"), Some(true));
/// ```
#[macro_export]
macro_rules! param_values {
    ($($key:expr => $value:expr),* $(,)?) => {{
        let mut values = $crate::values::ParameterValues::new();
        $(
            values.set($key, ::serde_json::json!($value));
        )*
        values
    }};
}

/// Create a parameter definition with less boilerplate.
///
/// # Examples
///
/// ```
/// use nebula_parameter::{param_def, prelude::*};
///
/// let p = param_def!(text "api_key", "API Key", required, sensitive);
/// let p = param_def!(number "timeout", "Timeout", default = 30.0);
/// let p = param_def!(checkbox "enabled", "Enabled");
/// ```
#[macro_export]
macro_rules! param_def {
    // text "key", "name"
    (text $key:expr, $name:expr) => {
        $crate::def::ParameterDef::Text($crate::types::TextParameter::new($key, $name))
    };

    // text "key", "name", required
    (text $key:expr, $name:expr, required) => {
        $crate::def::ParameterDef::Text($crate::types::TextParameter::new($key, $name).required())
    };

    // text "key", "name", sensitive
    (text $key:expr, $name:expr, sensitive) => {
        $crate::def::ParameterDef::Text($crate::types::TextParameter::new($key, $name).sensitive())
    };

    // text "key", "name", required, sensitive
    (text $key:expr, $name:expr, required, sensitive) => {
        $crate::def::ParameterDef::Text(
            $crate::types::TextParameter::new($key, $name)
                .required()
                .sensitive(),
        )
    };

    // number "key", "name"
    (number $key:expr, $name:expr) => {
        $crate::def::ParameterDef::Number($crate::types::NumberParameter::new($key, $name))
    };

    // number "key", "name", default = value
    (number $key:expr, $name:expr, default = $default:expr) => {
        $crate::def::ParameterDef::Number(
            $crate::types::NumberParameter::new($key, $name).default_value($default),
        )
    };

    // checkbox "key", "name"
    (checkbox $key:expr, $name:expr) => {
        $crate::def::ParameterDef::Checkbox($crate::types::CheckboxParameter::new($key, $name))
    };

    // secret "key", "name"
    (secret $key:expr, $name:expr) => {
        $crate::def::ParameterDef::Secret($crate::types::SecretParameter::new($key, $name))
    };
}
