//! Typed environment-variable reader with uniform error reporting.
//!
//! One parsing contract for the whole workspace: `var`/`var_opt` for strings,
//! `parse`/`parse_or` for `FromStr` types, `flag`/`flag_or` for booleans, and
//! `list` for delimited values. Variable names are `&str` so dynamic,
//! prefix-built names (e.g. per-provider OAuth vars) work uniformly.

use std::str::FromStr;

use crate::error::EnvError;

/// Read a required variable. `Err` if unset or not valid Unicode.
pub fn var(name: &str) -> Result<String, EnvError> {
    match std::env::var(name) {
        Ok(value) => Ok(value),
        Err(std::env::VarError::NotPresent) => Err(EnvError::Missing {
            var: name.to_owned(),
        }),
        Err(std::env::VarError::NotUnicode(_)) => Err(EnvError::NotUnicode {
            var: name.to_owned(),
        }),
    }
}

/// Read an optional variable. `Ok(None)` if unset; `Err` only if it is set
/// but not valid Unicode.
pub fn var_opt(name: &str) -> Result<Option<String>, EnvError> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(EnvError::NotUnicode {
            var: name.to_owned(),
        }),
    }
}

/// Parse a variable via [`FromStr`] (after trimming). `Ok(None)` if unset.
pub fn parse<T>(name: &str) -> Result<Option<T>, EnvError>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    match var_opt(name)? {
        None => Ok(None),
        Some(raw) => raw
            .trim()
            .parse()
            .map(Some)
            .map_err(|err: T::Err| EnvError::Parse {
                var: name.to_owned(),
                message: err.to_string(),
            }),
    }
}

/// Parse a variable via [`FromStr`], falling back to `default` when unset.
pub fn parse_or<T>(name: &str, default: T) -> Result<T, EnvError>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    Ok(parse(name)?.unwrap_or(default))
}

/// Parse a boolean. Accepts (case-insensitive) `true/1/yes/on` and
/// `false/0/no/off`. `Ok(None)` if unset; `Err` on any other value.
pub fn flag(name: &str) -> Result<Option<bool>, EnvError> {
    let Some(raw) = var_opt(name)? else {
        return Ok(None);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(Some(true)),
        "false" | "0" | "no" | "off" => Ok(Some(false)),
        _ => Err(EnvError::Invalid {
            var: name.to_owned(),
            value: raw,
            expected: "true|false|1|0|yes|no|on|off",
        }),
    }
}

/// Parse a boolean, falling back to `default` when unset.
pub fn flag_or(name: &str, default: bool) -> Result<bool, EnvError> {
    Ok(flag(name)?.unwrap_or(default))
}

/// Split a variable on whitespace and commas, dropping empties. Returns an
/// empty `Vec` when unset (operator-friendly list parsing).
#[must_use]
pub fn list(name: &str) -> Vec<String> {
    std::env::var(name)
        .unwrap_or_default()
        .split(|c: char| c.is_whitespace() || c == ',')
        .map(str::trim)
        .filter(|piece| !piece.is_empty())
        .map(str::to_owned)
        .collect()
}
