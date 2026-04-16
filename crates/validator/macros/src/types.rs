//! Shared type-introspection helpers used by both `parse` and `emit`.
//!
//! These utilities inspect `syn::Type` nodes to recognise the common
//! patterns the derive macro cares about: `Option<T>`, `Vec<T>`, `String`,
//! `bool`. Keeping them in one place avoids drift between phases.

use syn::Type;

/// Extract the inner type from `Option<T>`.
///
/// Returns `None` when `ty` is not a path type whose last segment is `Option`
/// with a single angle-bracketed type argument.
pub(crate) fn option_inner_type(ty: &Type) -> Option<&Type> {
    inner_generic_arg(ty, "Option")
}

/// Extract the inner type from `Vec<T>`.
///
/// Returns `None` when `ty` is not a path type whose last segment is `Vec`
/// with a single angle-bracketed type argument.
pub(crate) fn vec_inner_type(ty: &Type) -> Option<&Type> {
    inner_generic_arg(ty, "Vec")
}

/// Returns `true` when `ty` is a path type ending in `String`.
pub(crate) fn is_string_type(ty: &Type) -> bool {
    last_segment_is(ty, "String")
}

/// Returns `true` when `ty` is a path type ending in `bool`.
pub(crate) fn is_bool_type(ty: &Type) -> bool {
    last_segment_is(ty, "bool")
}

/// Returns `true` when `ty` is a path type ending in `Option`.
pub(crate) fn is_option_type(ty: &Type) -> bool {
    last_segment_is(ty, "Option")
}

/// Returns `true` when the last segment of `ty` matches `name` exactly.
fn last_segment_is(ty: &Type, name: &str) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    type_path
        .path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == name)
}

/// Returns the first generic argument of a path whose last segment matches
/// `outer` (e.g. `Option<T>` → `Some(T)`, `Vec<T>` → `Some(T)`).
fn inner_generic_arg<'a>(ty: &'a Type, outer: &str) -> Option<&'a Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident != outer {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    let syn::GenericArgument::Type(inner) = args.args.first()? else {
        return None;
    };
    Some(inner)
}
