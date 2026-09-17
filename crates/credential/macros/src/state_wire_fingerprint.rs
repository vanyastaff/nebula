//! `#[derive(StateWireFingerprint)]` macro implementation.
//!
//! Emits `impl StateWireFingerprint` with a `const SCHEMA_FINGERPRINT: u64`
//! computed at macro-expansion time (host side) over the state type's serde
//! **wire shape only** — never authored annotations, labels, descriptions, or
//! field values. A doc edit must not flip the fingerprint; a field rename,
//! type change, order change, or Option-ness change must.
//!
//! # Projection contract
//!
//! For each field in declaration order, the projection appends
//! `<field name as authored>\n<req|opt>\n<type tokens as written>\n`.
//!
//! - Field name: the Rust identifier as authored (serde renames are NOT
//!   captured — the projection is identity, not the serialized key spelling).
//! - Option-ness: `opt` when the field's type path's final segment is
//!   `Option` (syntactic check; `std::option::Option` spelled fully is caught
//!   the same way), else `req`.
//! - Type tokens: the field type's token stream rendered as written
//!   (`syn` token rendering — deterministic across builds/toolchains;
//!   whitespace normalization is fine, identity is what matters).
//!
//! Entries are separated by `\n`; identifiers and rendered type tokens never
//! contain newlines, so the concatenation is collision-free.
//!
//! Excluded from the projection: fields marked `#[serde(skip)]`,
//! `#[serde(skip_serializing)]`, or `#[serde(skip_deserializing)]` (not on the
//! wire). Other serde effects (`rename`, `flatten`, `with`) are NOT captured
//! — the per-state pin tests in `nebula_credential::state_envelope` are the
//! backstop for the built-in types.
//!
//! The projection bytes are hashed with FNV-1a 64 (offset basis
//! `0xcbf29ce484222325`, prime `0x100000001b3`).
//!
//! # Supported shapes
//!
//! - Named-field structs: one entry per wire field.
//! - Unit structs: hash of the empty projection.
//! - Tuple structs, enums, unions: compile error — persisted credential state
//!   is a named-field record in this codebase and the derive refuses shapes it
//!   cannot project unambiguously.
//!
//! Generic types are supported: the emitted impl carries the original
//! generics plus a `where #ty: ::nebula_credential::CredentialState` bound
//! (the supertrait requirement, satisfied by the type's own `CredentialState`
//! impl).

use nebula_macro_support::diag;
use proc_macro::TokenStream;
use quote::{ToTokens, quote};
use syn::{Data, DeriveInput, Fields, Meta, parse_macro_input};

/// FNV-1a 64-bit offset basis.
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Entry point for `#[derive(StateWireFingerprint)]`.
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

    let projection = projection_string(&input)?;
    let fingerprint = fnv1a_64(projection.as_bytes());

    let combined_where = if let Some(where_clause) = where_clause {
        let predicates = &where_clause.predicates;
        quote! {
            where
                #struct_name #ty_generics: ::nebula_credential::CredentialState,
                #predicates
        }
    } else {
        quote! {
            where #struct_name #ty_generics: ::nebula_credential::CredentialState
        }
    };

    Ok(quote! {
        impl #impl_generics ::nebula_credential::StateWireFingerprint
            for #struct_name #ty_generics
        #combined_where
        {
            const SCHEMA_FINGERPRINT: u64 = #fingerprint;
        }
    })
}

/// Build the wire-shape projection string for the input type (see the module
/// docs for the exact per-field format).
fn projection_string(input: &DeriveInput) -> syn::Result<String> {
    let Data::Struct(data) = &input.data else {
        return Err(diag::error_spanned(
            &input.ident,
            "#[derive(StateWireFingerprint)] only supports structs — persisted credential \
             state is a named-field record",
        ));
    };

    let fields = match &data.fields {
        Fields::Unit => return Ok(String::new()),
        Fields::Named(named) => &named.named,
        Fields::Unnamed(_) => {
            return Err(diag::error_spanned(
                &input.ident,
                "#[derive(StateWireFingerprint)] does not support tuple structs — the wire \
                 shape is ambiguous without field names",
            ));
        },
    };

    let mut projection = String::new();
    for field in fields {
        if field_is_serde_skipped(field) {
            continue;
        }
        let Some(name) = &field.ident else {
            return Err(diag::error_spanned(
                field,
                "#[derive(StateWireFingerprint)] cannot project an unnamed field",
            ));
        };
        let name = name.to_string();
        let optionality = if type_is_option(&field.ty) {
            "opt"
        } else {
            "req"
        };
        let type_tokens = field.ty.to_token_stream().to_string();

        projection.push_str(&name);
        projection.push('\n');
        projection.push_str(optionality);
        projection.push('\n');
        projection.push_str(&type_tokens);
        projection.push('\n');
    }

    Ok(projection)
}

/// Whether the field carries a serde skip marker (`skip`,
/// `skip_serializing`, or `skip_deserializing`) — i.e. it is not (fully) on
/// the wire.
fn field_is_serde_skipped(field: &syn::Field) -> bool {
    field.attrs.iter().any(|attr| {
        if !attr.path().is_ident("serde") {
            return false;
        }
        let Ok(Meta::List(list)) = attr.parse_args::<Meta>() else {
            return false;
        };
        let Ok(nested) = list
            .parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated)
        else {
            return false;
        };
        nested.iter().any(|meta| {
            let path = meta.path();
            path.is_ident("skip")
                || path.is_ident("skip_serializing")
                || path.is_ident("skip_deserializing")
        })
    })
}

/// Whether the type path's final segment is `Option` (syntactic check).
fn type_is_option(ty: &syn::Type) -> bool {
    matches!(
        ty,
        syn::Type::Path(type_path)
            if type_path
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "Option")
    )
}

/// FNV-1a 64 over the projection bytes (const-fn; the value is emitted as the
/// fingerprint const literal).
const fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    hash
}
