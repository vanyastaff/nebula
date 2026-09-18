//! Method-call aliases onto the builtin registry.
//!
//! Methods are a second call syntax for the same standard library: the
//! receiver becomes the first argument, so `items.filter(x => x > 1)` and
//! `filter(items, x => x > 1)` are the same call. This module only maps
//! JavaScript/Luxon-style names (`toUpperCase`, `includes`, `plus`) onto the
//! canonical builtin names; it must never grow its own implementations.
//!
//! Property-like members (`length`, `year`, `month`, …) are not aliases and
//! live in the evaluator's member access, because they are not calls.

/// How a method name maps onto a builtin call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MethodTarget {
    /// The receiver becomes the first argument.
    ///
    /// `items.filter(f)` → `filter(items, f)`.
    Receiver(&'static str),
    /// JavaScript `reduce` order: `items.reduce(fn, init)` →
    /// `reduce(items, init, fn)`.
    ReceiverReduce(&'static str),
}

impl MethodTarget {
    /// The canonical builtin name this target calls.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Receiver(name) | Self::ReceiverReduce(name) => name,
        }
    }
}

/// Resolve a method name to its canonical builtin call plan.
///
/// Returns `None` for names with no alias, which means "call the registered
/// builtin with the author's spelling" — canonical names (`filter`, `trim`, …)
/// take that path, and so does an unknown name, which the registry then reports
/// as not-found under its original spelling.
pub(crate) fn resolve_method(name: &str) -> Option<MethodTarget> {
    Some(match name {
        // JavaScript string/array methods onto the standard library.
        "toUpperCase" | "to_upper" => MethodTarget::Receiver("uppercase"),
        "toLowerCase" | "to_lower" => MethodTarget::Receiver("lowercase"),
        "includes" => MethodTarget::Receiver("contains"),
        "startsWith" => MethodTarget::Receiver("starts_with"),
        "endsWith" => MethodTarget::Receiver("ends_with"),
        "padStart" => MethodTarget::Receiver("pad_start"),
        "padEnd" => MethodTarget::Receiver("pad_end"),
        "toString" => MethodTarget::Receiver("to_string"),
        "toNumber" => MethodTarget::Receiver("to_number"),
        "toBoolean" => MethodTarget::Receiver("to_boolean"),
        "indexOf" => MethodTarget::Receiver("index_of"),
        "findIndex" => MethodTarget::Receiver("find_index"),
        "flatMap" => MethodTarget::Receiver("flat_map"),
        "groupBy" => MethodTarget::Receiver("group_by"),
        "substr" => MethodTarget::Receiver("substring"),
        // JavaScript `reduce(fn, init)` reverses the builtin's `(initial, fn)`.
        "reduce" => MethodTarget::ReceiverReduce("reduce"),

        // Luxon-style date methods. The receiver becomes the first argument,
        // which matches each builtin's `(value, …)` signature.
        "plus" | "add" => MethodTarget::Receiver("date_add"),
        "minus" | "subtract" => MethodTarget::Receiver("date_subtract"),
        "diff" => MethodTarget::Receiver("date_diff"),
        "toFormat" | "format" => MethodTarget::Receiver("format_date"),
        "toISO" => MethodTarget::Receiver("now_iso"),

        // Canonical names and everything else use the author's spelling.
        _ => return None,
    })
}

/// Resolve a namespace-qualified call (`Math.max(1, 2)`) to a builtin name.
///
/// Namespace members are ordinary builtins with no receiver argument, so
/// `Math.max(1, 2)` and `max(1, 2)` are the same call. Returns `None` when
/// the pair is not a known namespace member.
pub(crate) fn resolve_namespace(namespace: &str, method: &str) -> Option<&'static str> {
    match (namespace, method) {
        ("Math", "max") => Some("max"),
        ("Math", "min") => Some("min"),
        ("Math", "abs") => Some("abs"),
        ("Math", "round") => Some("round"),
        ("Math", "floor") => Some("floor"),
        ("Math", "ceil") => Some("ceil"),
        ("Math", "sqrt") => Some("sqrt"),
        ("Math", "pow") => Some("pow"),
        ("JSON", "parse") => Some("parse_json"),
        ("JSON", "stringify") => Some("to_json"),
        ("Number", "parseFloat" | "parseInt") => Some("to_number"),
        ("Object", "keys") => Some("keys"),
        ("Object", "values") => Some("values"),
        ("Object", "entries") => Some("entries"),
        ("Object", "fromEntries") => Some("from_entries"),
        ("Object", "assign") => Some("merge"),
        ("Array", "isArray") => Some("is_array"),
        _ => None,
    }
}

/// The value of a property-like member without a call.
///
/// `length`, `size`, `count`, and the date field getters are properties in the
/// n8n surface. Returns `None` when the member is not a known property of the
/// receiver's type, so callers can fall back to object lookup and then to the
/// missing-lookup policy.
pub(crate) fn member_property(
    receiver: &crate::RuntimeValue,
    name: &str,
) -> Option<crate::RuntimeValue> {
    use crate::RuntimeValue;

    match (receiver, name) {
        // `length` mirrors `length()`: Unicode scalar values for strings,
        // element count for arrays, key count for objects.
        (RuntimeValue::String(text), "length") => {
            Some(RuntimeValue::Integer(crate::value_utils::char_count(text)))
        },
        (RuntimeValue::Array(values), "length") => Some(RuntimeValue::Integer(values.len() as i64)),
        (RuntimeValue::Object(entries), "length") => {
            Some(RuntimeValue::Integer(entries.len() as i64))
        },
        // Date field getters, matching the `date_*` builtins.
        (RuntimeValue::DateTime(dt), "year") => {
            Some(RuntimeValue::Integer(i64::from(chrono::Datelike::year(dt))))
        },
        (RuntimeValue::DateTime(dt), "month") => Some(RuntimeValue::Integer(i64::from(
            chrono::Datelike::month(dt),
        ))),
        (RuntimeValue::DateTime(dt), "day") => {
            Some(RuntimeValue::Integer(i64::from(chrono::Datelike::day(dt))))
        },
        (RuntimeValue::DateTime(dt), "hour") => {
            Some(RuntimeValue::Integer(i64::from(chrono::Timelike::hour(dt))))
        },
        (RuntimeValue::DateTime(dt), "minute") => Some(RuntimeValue::Integer(i64::from(
            chrono::Timelike::minute(dt),
        ))),
        (RuntimeValue::DateTime(dt), "second") => Some(RuntimeValue::Integer(i64::from(
            chrono::Timelike::second(dt),
        ))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_names_use_the_authors_spelling() {
        assert_eq!(resolve_method("filter"), None);
        assert_eq!(
            resolve_method("reduce"),
            Some(MethodTarget::ReceiverReduce("reduce"))
        );
        assert_eq!(
            resolve_method("toUpperCase"),
            Some(MethodTarget::Receiver("uppercase"))
        );
    }

    #[test]
    fn javascript_names_map_to_one_library() {
        assert_eq!(
            resolve_method("includes"),
            Some(MethodTarget::Receiver("contains"))
        );
        assert_eq!(
            resolve_method("startsWith"),
            Some(MethodTarget::Receiver("starts_with"))
        );
    }

    #[test]
    fn namespace_members_map_to_receiverless_builtins() {
        assert_eq!(resolve_namespace("Math", "max"), Some("max"));
        assert_eq!(resolve_namespace("JSON", "parse"), Some("parse_json"));
        assert_eq!(resolve_namespace("JSON", "stringify"), Some("to_json"));
        assert_eq!(resolve_namespace("Object", "keys"), Some("keys"));
        assert_eq!(resolve_namespace("Math", "nope"), None);
    }

    #[test]
    fn date_methods_map_to_the_date_builtins() {
        assert_eq!(
            resolve_method("plus"),
            Some(MethodTarget::Receiver("date_add"))
        );
        assert_eq!(
            resolve_method("diff"),
            Some(MethodTarget::Receiver("date_diff"))
        );
        assert_eq!(
            resolve_method("toFormat"),
            Some(MethodTarget::Receiver("format_date"))
        );
    }

    #[test]
    fn property_members_do_not_need_a_call() {
        let text = crate::RuntimeValue::string("über");
        assert_eq!(
            member_property(&text, "length"),
            Some(crate::RuntimeValue::Integer(4))
        );
        let array = crate::RuntimeValue::array(vec![crate::RuntimeValue::Null]);
        assert_eq!(
            member_property(&array, "length"),
            Some(crate::RuntimeValue::Integer(1))
        );
        assert_eq!(member_property(&text, "year"), None);
    }
}
