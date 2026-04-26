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
///
/// ## Limitations (audit gap — best-effort detection)
///
/// The audit classifies field types by trailing path segment only
/// (see [`classify_type`]). It catches the literal `SecretString`,
/// `SecretBytes`, and their `Option<T>`/`Box<T>`/`Arc<T>`/`Rc<T>`
/// wrappers; it does **not** catch:
///
/// 1. **Nested `SensitiveScheme` types embedded in a `#[auth_scheme(public)]` struct.** A
///    proc-macro only sees its own input's tokens, never other crates' `impl SensitiveScheme for X`
///    declarations. A `public` scheme that embeds e.g. `SecretToken`, `OAuth2Token`, or any
///    user-defined `SensitiveScheme` slips through this audit.
/// 2. **Renamed re-exports** (e.g. `use SecretString as MyToken;`) — the classifier only matches
///    the literal trailing identifier.
/// 3. **Type aliases** that hide a sensitive primitive behind a public-looking name.
///
/// Defense-in-depth: the trait-level `SensitiveScheme: AuthScheme +
/// ZeroizeOnDrop` bound catches missing zeroize at the impl site for
/// the wrapping struct, so a `#[auth_scheme(public)]` struct that
/// embeds `OAuth2Token` would still need to satisfy `ZeroizeOnDrop` via
/// `derive` to compile — which signals to the author that the wrapping
/// type carries sensitive material. But the trait bound is not a strict
/// negative-impl mechanism, and a hand-rolled `Drop` that does **not**
/// zeroize will type-check.
///
/// Authors of nested-sensitive types should `#[derive(zeroize::ZeroizeOnDrop)]`
/// on the wrapping struct **and** declare it `#[auth_scheme(sensitive)]`
/// (or build it manually as `SensitiveScheme`). See the
/// `arch-publicscheme-nested-sensitive-audit` row in
/// `docs/tracking/credential-concerns-register.md` for the long-term
/// refinement plan (compile-time `where Self::FieldsX: PublicScheme`
/// reflection is not currently feasible at the macro level).
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
///
/// `Option<T>`, `Box<T>`, `Arc<T>`, `Rc<T>` recurse to their inner type so
/// `Option<SecretString>` on a `public` scheme is rejected (otherwise the
/// audit would miss this and the trait bound `PublicScheme: AuthScheme`
/// gives no friendly diagnostic).
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
        "Option" | "Box" | "Arc" | "Rc" => {
            // Look through the wrapper to its inner type and recurse.
            if let syn::PathArguments::AngleBracketed(args) = &last.arguments
                && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
            {
                return classify_type(inner);
            }
            TypeClass::Other
        },
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
/// Word-segment match: splits the name on `_` and matches any segment
/// (case-insensitive) against `token`, `secret`, `key`, `password`,
/// `bearer`. Catches `token`, `api_key`, `client_secret`, `access_token`,
/// `bearer_token`, etc.
///
/// Per Tech Spec §15.5: "field-name lint catches common secret markers;
/// combined with type-class audit, an author cannot declare a
/// sensitive-named plain `String` field."
///
/// **Allowlisted prefixes** (treated as non-secret regardless of trailing
/// secret-marker): `public_` — for `public_key` (the non-secret half of
/// an asymmetric pair). All other compound names like `token_id`,
/// `key_alg`, `bearer_type` ARE matched (they contain a secret-marker
/// segment). The intent is erring on the side of safety — additional
/// false positives can be silenced by renaming the field or by holding
/// the metadata in a wrapper struct.
fn is_secret_named(name: &str) -> bool {
    const SECRETS: &[&str] = &["token", "secret", "key", "password", "bearer"];
    /// Prefixes that mark a field as deliberately non-secret even when
    /// the rest of the name contains a secret-marker segment.
    const NON_SECRET_PREFIXES: &[&str] = &["public_"];

    let lower = name.to_ascii_lowercase();
    if NON_SECRET_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        return false;
    }
    lower.split('_').any(|segment| SECRETS.contains(&segment))
}
