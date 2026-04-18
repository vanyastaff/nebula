//! Rust-type → nebula-schema field kind mapping.
//!
//! The derive path needs two decisions per struct field:
//!
//! 1. Which `Field::*` constructor to emit.
//! 2. Whether to mark the resulting field `required` by default (i.e. is it wrapped in
//!    `Option<T>`?).
//!
//! Both answers fall out of syntactic inspection of the `syn::Type`. The
//! matcher looks at the last path segment of the type (so `std::string::String`
//! and plain `String` both resolve to the same bucket) and recognises a fixed
//! vocabulary of standard-library primitives plus the `Option<_>` / `Vec<_>`
//! wrappers. Unknown types fall through to [`FieldKind::UserDefined`], which
//! the derive layer uses to require the type to implement
//! [`HasSchema`](nebula_schema::HasSchema).

use syn::{GenericArgument, PathArguments, Type, TypePath};

/// Classification of a Rust type for schema-derivation purposes.
#[derive(Clone)]
pub(crate) enum FieldKind {
    String,
    Boolean,
    IntegerNumber,
    FloatNumber,
    /// `Option<T>` — marks the field optional and recurses into `T`.
    Optional(Box<FieldKind>),
    /// `Vec<T>` — emits a `ListField` whose item is the inner kind.
    List(Box<FieldKind>),
    /// Any other concrete type. The derive uses this as the signal to call
    /// `<T as HasSchema>::schema()` at runtime (Task 10 blanket object
    /// inference).
    UserDefined(Box<Type>),
    /// Unsupported integer width — the schema layer's rules only handle
    /// values that fit in `i64` / `u64` (via `serde_json::Number`), so
    /// wider integer types like `i128`/`u128`/`isize`/`usize` are
    /// rejected at macro expansion rather than silently mapped to
    /// `IntegerNumber` and breaking at runtime.
    UnsupportedInteger(String),
}

impl FieldKind {
    pub fn is_optional(&self) -> bool {
        matches!(self, FieldKind::Optional(_))
    }

    pub fn inner(&self) -> &FieldKind {
        match self {
            FieldKind::Optional(inner) => inner,
            other => other,
        }
    }
}

/// Inspect a `syn::Type` and return its [`FieldKind`] classification.
pub(crate) fn classify(ty: &Type) -> FieldKind {
    if let Some(inner) = unwrap_single_generic(ty, "Option") {
        return FieldKind::Optional(Box::new(classify(inner)));
    }
    if let Some(inner) = unwrap_single_generic(ty, "Vec") {
        return FieldKind::List(Box::new(classify(inner)));
    }
    match last_segment_ident(ty) {
        Some(name) => match name.as_str() {
            "String" | "str" => FieldKind::String,
            "bool" => FieldKind::Boolean,
            // Narrow integer types that round-trip cleanly through
            // `serde_json::Number` (via `From<i64>` / `From<u64>`).
            "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => FieldKind::IntegerNumber,
            "f32" | "f64" => FieldKind::FloatNumber,
            // Wider integer widths — no `Number::from_i128/u128` impl
            // without the `serde_json/arbitrary_precision` feature, and
            // the validator's rule layer is `i64`-bounded anyway.
            "i128" | "u128" | "isize" | "usize" => FieldKind::UnsupportedInteger(name),
            _ => FieldKind::UserDefined(Box::new(ty.clone())),
        },
        None => FieldKind::UserDefined(Box::new(ty.clone())),
    }
}

/// Return the final path segment identifier of a type, if any.
///
/// Handles both bare idents (`String`, `bool`) and fully-qualified paths
/// (`std::string::String`).
fn last_segment_ident(ty: &Type) -> Option<String> {
    if let Type::Path(TypePath { qself: None, path }) = ty {
        path.segments.last().map(|s| s.ident.to_string())
    } else {
        None
    }
}

/// If `ty` is `Wrapper<Inner>`, return the inner type.
fn unwrap_single_generic<'t>(ty: &'t Type, wrapper: &str) -> Option<&'t Type> {
    if let Type::Path(TypePath { qself: None, path }) = ty
        && let Some(seg) = path.segments.last()
        && seg.ident == wrapper
        && let PathArguments::AngleBracketed(args) = &seg.arguments
        && let Some(GenericArgument::Type(inner)) = args.args.first()
    {
        return Some(inner);
    }
    None
}
