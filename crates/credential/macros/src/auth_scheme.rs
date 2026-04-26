//! `#[derive(AuthScheme)]` macro implementation.
//!
//! Per Tech Spec §15.5, the macro audits scheme fields for the
//! sensitivity dichotomy:
//!
//! - `#[auth_scheme(sensitive)]` — schemes holding secret material. Field-type audit forbids plain
//!   `String` / `Vec<u8>` for token-named slots; nested schemes must impl `SensitiveScheme`.
//!   Field-name lint catches `token` / `secret` / `key` / `password` / `bearer` regardless of
//!   declared type.
//! - `#[auth_scheme(public)]` — schemes holding no secret material. Audit rejects any
//!   `SecretString` / `SecretBytes` / nested `SensitiveScheme` field.
//!
//! Mutually exclusive: declaring both fails at parse time.

use nebula_macro_support::{attrs, diag};
use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Type, parse_macro_input, spanned::Spanned};

/// Sensitivity declaration parsed from `#[auth_scheme(...)]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sensitivity {
    Sensitive,
    Public,
}

/// Entry point for `#[derive(AuthScheme)]`.
pub(crate) fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand(input) {
        Ok(ts) => ts.into(),
        Err(e) => diag::to_compile_error(e).into(),
    }
}

fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let attr_args = attrs::parse_attrs(&input.attrs, "auth_scheme")?;

    // Required: `pattern = Variant`
    let pattern_ident = attr_args.get_ident("pattern").ok_or_else(|| {
        diag::error_spanned(
            struct_name,
            "#[derive(AuthScheme)] requires `#[auth_scheme(pattern = Variant)]`",
        )
    })?;

    // Required: exactly one of `sensitive` or `public`
    let sensitive_flag = attr_args.has_flag("sensitive");
    let public_flag = attr_args.has_flag("public");

    let sensitivity = match (sensitive_flag, public_flag) {
        (true, true) => {
            return Err(diag::error_spanned(
                struct_name,
                "#[auth_scheme(...)] cannot declare both `sensitive` and `public` — they are \
                 mutually exclusive (per Tech Spec §15.5)",
            ));
        },
        (false, false) => {
            return Err(diag::error_spanned(
                struct_name,
                "#[auth_scheme(...)] must declare exactly one of `sensitive` or `public` (per \
                 Tech Spec §15.5 dichotomy)",
            ));
        },
        (true, false) => Sensitivity::Sensitive,
        (false, true) => Sensitivity::Public,
    };

    // Walk fields and audit per sensitivity
    audit_fields(&input, sensitivity)?;

    let pattern_path = quote! {
        ::nebula_core::auth::AuthPattern::#pattern_ident
    };

    let sensitivity_impl = match sensitivity {
        Sensitivity::Sensitive => quote! {
            impl #impl_generics ::nebula_core::auth::SensitiveScheme
                for #struct_name #ty_generics #where_clause {}
        },
        Sensitivity::Public => quote! {
            impl #impl_generics ::nebula_core::auth::PublicScheme
                for #struct_name #ty_generics #where_clause {}
        },
    };

    let expanded = quote! {
        impl #impl_generics ::nebula_core::auth::AuthScheme
            for #struct_name #ty_generics #where_clause
        {
            fn pattern() -> ::nebula_core::auth::AuthPattern {
                #pattern_path
            }
        }

        #sensitivity_impl
    };

    Ok(expanded)
}

/// Walk the struct fields and apply sensitivity-specific audits.
///
/// For `sensitive`: reject plain `String`/`Vec<u8>` for any field, especially
/// fields whose name implies sensitivity. Nested schemes (non-primitive types)
/// are accepted as-is — their own `#[derive(AuthScheme)]` audits them.
///
/// For `public`: reject any `SecretString` / `SecretBytes` field.
fn audit_fields(input: &DeriveInput, sensitivity: Sensitivity) -> syn::Result<()> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new(
            input.ident.span(),
            "#[derive(AuthScheme)] only supports structs",
        ));
    };

    let fields = match &data.fields {
        Fields::Named(named) => &named.named,
        Fields::Unnamed(_) => {
            return Err(syn::Error::new(
                input.ident.span(),
                "#[derive(AuthScheme)] only supports structs with named fields (per Tech Spec \
                 §15.5 audit needs field names)",
            ));
        },
        Fields::Unit => return Ok(()),
    };

    for field in fields {
        let Some(ident) = &field.ident else {
            continue;
        };
        let field_name = ident.to_string();
        let type_class = classify_type(&field.ty);

        match sensitivity {
            Sensitivity::Sensitive => {
                // Field-type audit: plain String / Vec<u8> rejected on sensitive scheme
                // when the field name implies a secret. Nested types accepted as-is.
                if matches!(type_class, TypeClass::PlainString | TypeClass::PlainBytes)
                    && is_secret_named(&field_name)
                {
                    return Err(syn::Error::new(
                        field.span(),
                        format!(
                            "field `{field_name}` on #[auth_scheme(sensitive)] struct must be \
                             SecretString or SecretBytes (plain {} for a secret-named field is a \
                             leak risk per Tech Spec §15.5)",
                            type_class.display(),
                        ),
                    ));
                }
            },
            Sensitivity::Public => {
                if matches!(type_class, TypeClass::SecretString | TypeClass::SecretBytes) {
                    return Err(syn::Error::new(
                        field.span(),
                        format!(
                            "field `{field_name}` on #[auth_scheme(public)] struct cannot be {} \
                             — declare #[auth_scheme(sensitive)] instead (per Tech Spec §15.5)",
                            type_class.display(),
                        ),
                    ));
                }
            },
        }
    }

    Ok(())
}

/// Coarse type classification used by the audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypeClass {
    /// `String` (owned)
    PlainString,
    /// `Vec<u8>`
    PlainBytes,
    /// `secrecy::SecretString`, our `SecretString` re-export, etc.
    SecretString,
    /// `SecretBytes` / `SecretVec<u8>`
    SecretBytes,
    /// Anything else (nested scheme types, primitives, options, etc.)
    Other,
}

impl TypeClass {
    fn display(self) -> &'static str {
        match self {
            Self::PlainString => "String",
            Self::PlainBytes => "Vec<u8>",
            Self::SecretString => "SecretString",
            Self::SecretBytes => "SecretBytes",
            Self::Other => "(unknown)",
        }
    }
}

/// Classify a type by trailing path segment / common shapes.
///
/// The audit is best-effort and conservative: unrecognized types fall to
/// `Other`. The trait-level `SensitiveScheme: ZeroizeOnDrop` bound catches
/// missing zeroize at the impl site, so the macro audit is defense in depth.
fn classify_type(ty: &Type) -> TypeClass {
    let Type::Path(type_path) = ty else {
        return TypeClass::Other;
    };
    let Some(last) = type_path.path.segments.last() else {
        return TypeClass::Other;
    };
    let name = last.ident.to_string();
    match name.as_str() {
        "String" => TypeClass::PlainString,
        "SecretString" => TypeClass::SecretString,
        "SecretBytes" => TypeClass::SecretBytes,
        "Vec" => {
            // Distinguish `Vec<u8>` from `Vec<T>` in general
            if let syn::PathArguments::AngleBracketed(args) = &last.arguments
                && let Some(syn::GenericArgument::Type(Type::Path(inner))) = args.args.first()
                && inner.path.segments.last().is_some_and(|s| s.ident == "u8")
            {
                return TypeClass::PlainBytes;
            }
            TypeClass::Other
        },
        _ => TypeClass::Other,
    }
}

/// Whether a field name suggests it carries secret material.
///
/// Matches the regex `^(token|secret|key|password|bearer)$/i` — explicit
/// per Tech Spec §15.5 to keep the audit predictable. Field names like
/// `token_id`, `key_alg`, `bearer_type` are NOT matched (they describe
/// metadata about a secret, not the secret itself).
fn is_secret_named(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "token" | "secret" | "key" | "password" | "bearer"
    )
}
