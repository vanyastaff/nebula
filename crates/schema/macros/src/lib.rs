//! Compile-time macros for nebula-schema.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{DeriveInput, LitStr, parse_macro_input};

mod attrs;
mod codec_attrs;
mod derive_enum;
mod derive_enum_union;
mod derive_property_type;
mod derive_schema;
mod named_conditions;
mod schema_type;
mod type_infer;

#[cfg(test)]
mod codec_authoring_tests;

/// Canonical schema path; the shared final expansion pass resolves it for the
/// invoking crate.
pub(crate) fn crate_path() -> TokenStream2 {
    quote!(::nebula_schema)
}

/// Build a `FieldKey` (from `nebula-schema`) from a string literal, using the same rules as
/// `FieldKey::new` at **compile time** (non-empty, max 64 chars, ASCII identifier: leading letter
/// or `_`, then letters, digits, or `_`).
///
/// ```text
/// let k = field_key!("alpha");   // OK
/// let k = field_key!("1bad");    // compile error
/// ```
///
/// This crate is `proc-macro = true`, so it cannot depend on `nebula-schema`
/// and the snippet above cannot be a runnable doctest here. For a compiling
/// example see `nebula_schema::FieldKey` and the `field_key!` re-export in the
/// parent `nebula-schema` crate.
#[proc_macro]
pub fn field_key(input: TokenStream) -> TokenStream {
    let lit = parse_macro_input!(input as LitStr);
    let value = lit.value();

    if let Err(msg) = validate_field_key(&value) {
        return syn::Error::new(lit.span(), format!("invalid FieldKey literal: {msg}"))
            .to_compile_error()
            .into();
    }

    let crate_path = crate_path();

    let out = quote! {{
        const __NEBULA_KEY: #crate_path::__private::LiteralFieldKey =
            match #crate_path::__private::LiteralFieldKey::parse(#lit) {
                ::core::option::Option::Some(key) => key,
                ::core::option::Option::None => {
                    ::core::panic!("schema macro and runtime key validation disagree")
                }
            };
        #crate_path::__private::field_key_from_validated_literal(__NEBULA_KEY)
    }};
    nebula_macro_support::paths::resolve_generated_crate_paths(out).into()
}

/// Derive `HasSchema` (from `nebula-schema`).
///
/// Emits `PropertyType` as the authoritative structural graph description, without
/// claiming serde codec fidelity. The legacy `HasSchema` projection remains
/// available where the graph is exactly representable. Standalone `Schema`
/// retains its legacy generic and enum-shape restrictions; use `PropertyType`
/// or `schema_type` for per-instantiation generic graphs. Single-field
/// newtypes describe the wrapped root, including nullable and array roots.
/// Descriptor owners must be `'static`. Direct recursive records use the graph
/// and return an error when a legacy tree projection cannot represent recursion;
/// mutually recursive standalone derives retain the legacy cache restriction.
/// Use `schema_type(input|output|both)` to own serde and codec generation.
///
/// On a **struct** the schema is a record of typed fields. On an **enum** it is a
/// tagged union (`SchemaKind::Union`) — one variant per enum variant, honoring
/// serde's enum tagging (external by default, or adjacent via
/// `#[serde(tag = "..", content = "..")]`); internally-tagged and untagged enums,
/// and tuple variants with more than one field, are rejected. (To embed an enum as
/// a select *field* inside a struct, use `#[field(enum_select)]` +
/// `#[derive(EnumSelect)]` instead.)
///
/// Supported attributes:
/// - `#[property(...)]` — structured Phase-5 property sections:
///   `display(...)`, `input(...)`, and `validate(...)`.
/// - `#[field(...)]` — label/description/placeholder/default/hint/secret/
///   multiline/no_expression/expression_required/enum_select/skip/group.
/// - `#[validate(...)]` — required/length(min,max)/range(min..=max)/ pattern/url/email.
/// - `#[schema(...)]` — struct-level options: `custom = "..."` → the validator's
///   `Rule::custom` on the built schema (deferred wire hook); `reserved("a", "b")`
///   → keys that may not be used by any field (reusing a removed field's key would
///   misread older documents), rejected at expansion if a field's resolved key or
///   a `#[serde(alias = ..)]` collides.
///   `condition(name, C)` declares a checked named condition in the containing
///   record; `field(identifier)` resolves the canonical inbound serde key and
///   `root("/path")` uses an absolute pointer. Duplicate, undefined, recursive,
///   and oversized named expansions are rejected.
/// - `#[serde(...)]` — read for key alignment so the schema key equals the wire
///   key: `rename` / `rename_all` rename the field or variant, `skip` /
///   `skip_deserializing` drop it, `tag` / `content` select adjacent enum tagging.
///   `#[serde(flatten)]` is rejected (splicing is a follow-up).
#[proc_macro_derive(Schema, attributes(property, field, validate, schema, serde))]
pub fn derive_schema(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let tokens = expand_legacy_schema(&input).unwrap_or_else(|error| error.to_compile_error());
    nebula_macro_support::paths::resolve_generated_crate_paths(tokens).into()
}

fn expand_legacy_schema(input: &DeriveInput) -> syn::Result<TokenStream2> {
    // Legacy errors carry refusal contracts, not just unsupported lowering.
    // Validate those contracts before generating the additional graph impl.
    if input.generics.type_params().next().is_some()
        || input.generics.const_params().next().is_some()
    {
        return derive_schema::expand(input.clone());
    }
    let newtype = matches!(&input.data, syn::Data::Struct(data) if matches!(data.fields, syn::Fields::Unnamed(_)))
        || (matches!(input.data, syn::Data::Struct(_))
            && codec_attrs::CodecAttrs::parse(&input.attrs)?.transparent);
    let graph_projection = newtype || has_direct_recursion(input);
    let legacy = if newtype {
        derive_property_type::graph_has_schema(input)
    } else {
        let validated = derive_schema::expand(input.clone())?;
        if graph_projection {
            derive_property_type::graph_has_schema(input)
        } else {
            validated
        }
    };
    // Cached legacy HasSchema already verifies SecretInput. Keeping that check
    // in both impls duplicates diagnostics for a single invalid secret field.
    let property = derive_property_type::expand_legacy_schema(input, graph_projection)?;
    Ok(quote!(#property #legacy))
}

/// Derive a structural definition graph. This grants no serde codec witness.
#[proc_macro_derive(PropertyType, attributes(property, field, validate, schema, serde))]
pub fn derive_property_type(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let tokens =
        derive_property_type::expand(&input).unwrap_or_else(|error| error.to_compile_error());
    nebula_macro_support::paths::resolve_generated_crate_paths(tokens).into()
}

/// Own serde and schema generation for exactly `input`, `output`, or `both`.
#[proc_macro_attribute]
pub fn schema_type(options: TokenStream, input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let tokens =
        schema_type::expand(options.into(), input).unwrap_or_else(|error| error.to_compile_error());
    nebula_macro_support::paths::resolve_generated_crate_paths(tokens).into()
}

/// Derive `HasSelectOptions` (from `nebula-schema`) for a unit-only enum.
/// Variant names become catalog values following serde (`rename` / `rename_all`,
/// else `snake_case`); `#[serde(skip)]` drops a variant. Use
/// `#[field(label = "...")]` to override the display label.
#[proc_macro_derive(EnumSelect, attributes(property, field))]
pub fn derive_enum_select(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let tokens = derive_enum::expand(input).unwrap_or_else(|error| error.to_compile_error());
    nebula_macro_support::paths::resolve_generated_crate_paths(tokens).into()
}

/// The legacy cache cannot initialize a schema that reaches itself. The graph
/// registers the owner first; legacy discovery can then fail closed when that
/// recursive graph has no exact tree projection.
fn has_direct_recursion(input: &DeriveInput) -> bool {
    let fields = match &input.data {
        syn::Data::Struct(data) => data.fields.iter().collect::<Vec<_>>(),
        syn::Data::Enum(data) => data
            .variants
            .iter()
            .flat_map(|variant| variant.fields.iter())
            .collect(),
        syn::Data::Union(_) => return false,
    };
    fields
        .iter()
        .any(|field| refers_to_owner(&field.ty, &input.ident))
}

fn refers_to_owner(ty: &syn::Type, owner: &syn::Ident) -> bool {
    match ty {
        syn::Type::Path(path) => {
            path.qself.as_ref().is_some_and(|qualified| refers_to_owner(&qualified.ty, owner))
                || path.path.segments.iter().any(|segment| {
                    segment.ident == *owner || segment.ident == "Self"
                        || match &segment.arguments {
                            syn::PathArguments::AngleBracketed(arguments) => arguments.args.iter().any(|argument| {
                                matches!(argument, syn::GenericArgument::Type(ty) if refers_to_owner(ty, owner))
                            }),
                            _ => false,
                        }
                })
        },
        syn::Type::Array(array) => refers_to_owner(&array.elem, owner),
        syn::Type::Reference(reference) => refers_to_owner(&reference.elem, owner),
        syn::Type::Paren(parenthesis) => refers_to_owner(&parenthesis.elem, owner),
        syn::Type::Group(group) => refers_to_owner(&group.elem, owner),
        syn::Type::Tuple(tuple) => tuple.elems.iter().any(|ty| refers_to_owner(ty, owner)),
        _ => false,
    }
}

/// Validate a candidate schema field key against the `FieldKey` rules
/// (non-empty, ≤64 chars, leading ASCII letter or `_`, then ASCII alphanumerics
/// or `_`). Shared by the `field_key!` macro and the `Schema` / `EnumSelect`
/// derives so the rules live in exactly one place (mirror of
/// `nebula_schema::FieldKey::new`).
pub(crate) fn validate_field_key(value: &str) -> Result<(), &'static str> {
    if value.is_empty() {
        return Err("key cannot be empty");
    }
    if value.chars().count() > 64 {
        return Err("key max 64 chars");
    }
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err("key cannot be empty");
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err("key must start with letter or underscore");
    }
    for ch in chars {
        if !ch.is_ascii_alphanumeric() && ch != '_' {
            return Err("key must be ASCII alphanumeric or underscore");
        }
    }
    Ok(())
}
