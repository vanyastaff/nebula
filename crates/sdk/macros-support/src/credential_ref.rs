//! Phantom-shim rewrite helpers for `CredentialRef<dyn X>` field types.
//!
//! Centralised here (in the non-proc-macro support crate) so that
//! `nebula-action-macros` and any other macro crate that emits
//! action-shaped structs can share the same rewrite logic and exercise
//! it from unit tests. Proc-macro crates themselves cannot expose pub
//! helpers for integration tests; this crate can.
//!
//! See ADR-0035 paragraph 4.3 (action-side translation) and Tech Spec
//! 2.7 (`#[action]` macro translation, "rewrites silently") for the
//! contract.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Field, Fields, GenericArgument, Ident, ItemStruct, PathArguments, Type, TypeParamBound};

/// Rewrite every `CredentialRef<dyn X>` field on `item` to
/// `CredentialRef<dyn XPhantom>` in place.
///
/// Pattern 1 (`CredentialRef<ConcreteType>`) is pass-through: the
/// argument is not a trait-object, so no rewrite is applied. Fields
/// that don't reference `CredentialRef` at all are also pass-through.
pub fn rewrite_struct_credential_refs(item: &mut ItemStruct) {
    match &mut item.fields {
        Fields::Named(named) => {
            for field in &mut named.named {
                rewrite_field(field);
            }
        },
        Fields::Unnamed(unnamed) => {
            for field in &mut unnamed.unnamed {
                rewrite_field(field);
            }
        },
        Fields::Unit => {},
    }
}

fn rewrite_field(field: &mut Field) {
    rewrite_type(&mut field.ty);
}

/// Rewrite `ty` in place if it matches `CredentialRef<dyn X>`.
///
/// Match is by path-tail (last segment named `CredentialRef`) so the
/// helper accepts both `CredentialRef<...>` and fully-qualified
/// `nebula_credential::CredentialRef<...>` user-facing forms.
pub fn rewrite_type(ty: &mut Type) {
    let Type::Path(type_path) = ty else { return };
    let Some(last_segment) = type_path.path.segments.last_mut() else {
        return;
    };
    if last_segment.ident != "CredentialRef" {
        return;
    }
    let PathArguments::AngleBracketed(generic_args) = &mut last_segment.arguments else {
        return;
    };
    let Some(first_arg) = generic_args.args.first_mut() else {
        return;
    };
    let GenericArgument::Type(inner) = first_arg else {
        return;
    };
    let Type::TraitObject(trait_obj) = inner else {
        return;
    };

    for bound in &mut trait_obj.bounds {
        let TypeParamBound::Trait(trait_bound) = bound else {
            continue;
        };
        let Some(seg) = trait_bound.path.segments.last_mut() else {
            continue;
        };
        seg.ident = Ident::new(&format!("{}Phantom", seg.ident), seg.ident.span());
    }
}

/// Test helper - parse a struct from source, apply the rewrite, and
/// return the canonical token string of the result.
#[doc(hidden)]
pub fn rewrite_struct_to_string(src: &str) -> Result<String, String> {
    let mut item: ItemStruct = syn::parse_str(src).map_err(|e| e.to_string())?;
    rewrite_struct_credential_refs(&mut item);
    Ok(quote!(#item).to_string())
}

/// Test helper - parse a type from source, apply the rewrite, and
/// return the canonical token string of the result.
#[doc(hidden)]
pub fn rewrite_type_to_string(src: &str) -> Result<String, String> {
    let mut ty: Type = syn::parse_str(src).map_err(|e| e.to_string())?;
    rewrite_type(&mut ty);
    Ok(quote!(#ty).to_string())
}

#[doc(hidden)]
pub fn _emit_token_stream(item: &ItemStruct) -> TokenStream2 {
    quote!(#item)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern2_dyn_capability_rewrites_with_phantom_suffix() {
        let out = rewrite_type_to_string("CredentialRef<dyn BitbucketBearer>").unwrap();
        assert_eq!(
            out.replace(' ', ""),
            "CredentialRef<dynBitbucketBearerPhantom>"
        );
    }

    #[test]
    fn pattern2_with_send_sync_bounds_preserves_extra_bounds() {
        let out =
            rewrite_type_to_string("CredentialRef<dyn BitbucketBearer + Send + Sync>").unwrap();
        let canon = out.replace(' ', "");
        assert!(
            canon.contains("BitbucketBearerPhantom"),
            "expected phantom suffix; got: {out}"
        );
        assert!(
            canon.contains("Send") && canon.contains("Sync"),
            "expected Send + Sync preserved; got: {out}"
        );
    }

    #[test]
    fn pattern1_concrete_credential_passes_through_unchanged() {
        let src = "CredentialRef<SlackOAuth2Credential>";
        let out = rewrite_type_to_string(src).unwrap();
        assert_eq!(out.replace(' ', ""), src.replace(' ', ""));
    }

    #[test]
    fn fully_qualified_credential_ref_path_still_rewrites() {
        let out = rewrite_type_to_string("nebula_credential::CredentialRef<dyn BitbucketBearer>")
            .unwrap();
        assert!(
            out.replace(' ', "").contains("BitbucketBearerPhantom"),
            "fully-qualified path should still rewrite; got: {out}"
        );
    }

    #[test]
    fn non_credential_ref_type_is_pass_through() {
        let src = "Vec<dyn BitbucketBearer>";
        let out = rewrite_type_to_string(src).unwrap();
        assert_eq!(
            out.replace(' ', ""),
            src.replace(' ', ""),
            "non-CredentialRef wrappers must not be rewritten"
        );
    }

    #[test]
    fn struct_with_pattern2_field_rewrites_field_only() {
        let src = "struct A { #[credential] pub bb: CredentialRef<dyn BitbucketBearer>, pub other: String }";
        let out = rewrite_struct_to_string(src).unwrap();
        let canon = out.replace(' ', "");
        assert!(
            canon.contains("CredentialRef<dynBitbucketBearerPhantom>"),
            "Pattern 2 field should be rewritten; got: {out}"
        );
        assert!(
            canon.contains("other:String"),
            "non-credential field should be preserved verbatim; got: {out}"
        );
    }

    #[test]
    fn struct_with_pattern1_field_is_pass_through() {
        let src = "struct A { pub slack: CredentialRef<SlackOAuth2Credential> }";
        let out = rewrite_struct_to_string(src).unwrap();
        assert_eq!(
            out.replace(' ', ""),
            src.replace(' ', ""),
            "Pattern 1 (concrete) must remain unchanged"
        );
    }
}
