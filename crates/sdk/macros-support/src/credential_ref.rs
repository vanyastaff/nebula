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

/// Rewrite `ty` in place, recursively walking generic-argument trees so
/// that wrapper types (`Vec<CredentialRef<dyn X>>`,
/// `Option<CredentialRef<dyn X>>`, `Box<CredentialRef<dyn X>>`, …) get
/// the same Pattern 2 phantom-suffix treatment as the bare form.
///
/// Match is by path-tail (last segment named `CredentialRef`) so the
/// helper accepts both `CredentialRef<...>` and fully-qualified
/// `nebula_credential::CredentialRef<...>` user-facing forms.
///
/// Marker bounds (`Send`, `Sync`, `Sized`, `Unpin`, `Copy`, `Clone`) are
/// preserved verbatim — only the capability trait identifier receives
/// the `Phantom` suffix. Without this guard, `CredentialRef<dyn Cap +
/// Send + Sync>` would expand to nonsensical `SendPhantom` / `SyncPhantom`
/// references that fail with cryptic `E0405` and have no path back to
/// the macro for the plugin author. ADR-0035 §5 retains `Send + Sync` on
/// the phantom trait as a forward-compat promise; user-written
/// `dyn Cap + Send + Sync` must compose with that promise unchanged.
///
/// `Type::Reference` (e.g. `CredentialRef<&dyn Cap>`) is **out of scope**
/// today — the recursive walk only descends into `Type::Path` generics.
/// Adding a `Type::Reference` arm would let `CredentialRef<&dyn Cap>`
/// rewrite to `CredentialRef<&dyn CapPhantom>`, but that form is not a
/// supported Pattern 2 spelling per ADR-0035 §1 (canonical form is
/// owning `CredentialRef<dyn Cap>`). Authors who write the borrowed
/// form get rustc's normal `E0191` for the unspecified-assoc-type
/// closure on `dyn Cap` — which is the correct guidance to switch to
/// the canonical owning form.
pub fn rewrite_type(ty: &mut Type) {
    let Type::Path(type_path) = ty else { return };
    let Some(last_segment) = type_path.path.segments.last_mut() else {
        return;
    };

    // Recurse into ALL generic args first so wrapper types
    // (Vec / Option / Box / …) reach the inner CredentialRef.
    if let PathArguments::AngleBracketed(generic_args) = &mut last_segment.arguments {
        for arg in &mut generic_args.args {
            if let GenericArgument::Type(inner) = arg {
                rewrite_type(inner);
            }
        }
    }

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
        // Pass marker bounds (Send / Sync / Sized / Unpin / Copy / Clone)
        // through unchanged. Same list as
        // `nebula_credential_macros::capability::is_marker_bound`.
        if matches!(
            seg.ident.to_string().as_str(),
            "Send" | "Sync" | "Sized" | "Unpin" | "Copy" | "Clone"
        ) {
            continue;
        }
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
    fn pattern2_send_sync_bounds_preserve_marker_identity() {
        let out =
            rewrite_type_to_string("CredentialRef<dyn BitbucketBearer + Send + Sync>").unwrap();
        let canon = out.replace(' ', "");
        // Phantom-suffix only on the capability trait.
        assert!(
            canon.contains("BitbucketBearerPhantom"),
            "capability must be suffixed: {out}"
        );
        // Markers must NOT be suffixed (regression guard for the
        // SendPhantom / SyncPhantom mangling bug — substring containment
        // alone would silently pass since `SendPhantom` contains "Send").
        assert!(
            !canon.contains("SendPhantom"),
            "Send must not be suffixed: {out}"
        );
        assert!(
            !canon.contains("SyncPhantom"),
            "Sync must not be suffixed: {out}"
        );
        // Markers must remain as standalone tokens.
        assert!(canon.contains("+Send"), "Send must remain: {out}");
        assert!(canon.contains("+Sync"), "Sync must remain: {out}");
    }

    #[test]
    fn pattern2_full_marker_bound_list_passes_through_unchanged() {
        // Lock in the full marker list from `is_marker_bound` so any
        // future drift from the canonical set is caught immediately.
        // `Sized` / `Unpin` / `Copy` / `Clone` are not all valid trait-object
        // bounds in production code, but the rewriter must still treat them
        // as markers — exercising via `rewrite_type_to_string` lets the
        // syn parser tolerate the synthetic combination.
        let out = rewrite_type_to_string(
            "CredentialRef<dyn BitbucketBearer + Send + Sync + Unpin + Sized + Copy + Clone>",
        )
        .unwrap();
        let canon = out.replace(' ', "");
        assert!(
            canon.contains("BitbucketBearerPhantom"),
            "capability must be suffixed: {out}"
        );
        for marker in ["Send", "Sync", "Unpin", "Sized", "Copy", "Clone"] {
            let mangled = format!("{marker}Phantom");
            assert!(
                !canon.contains(&mangled),
                "marker `{marker}` must not be suffixed; got `{mangled}` in: {out}"
            );
        }
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

    // --- I4 wrapper-walk regression guards -----------------------------------
    //
    // Wrapper types around `CredentialRef<dyn X>` must reach the inner
    // capability and apply the phantom suffix. The pre-fix rewriter only
    // matched the outermost segment, so authors who wrote
    // `Vec<CredentialRef<dyn Cap>>` got a struct that compiled without
    // phantom enforcement — silently bypassing the ADR-0035 guarantee.

    #[test]
    fn pattern2_inside_vec_rewrites() {
        let out = rewrite_type_to_string("Vec<CredentialRef<dyn BitbucketBearer>>").unwrap();
        let canon = out.replace(' ', "");
        assert!(
            canon.contains("CredentialRef<dynBitbucketBearerPhantom"),
            "Vec wrapper should rewrite inner: {out}"
        );
    }

    #[test]
    fn pattern2_inside_option_rewrites() {
        let out = rewrite_type_to_string("Option<CredentialRef<dyn BitbucketBearer>>").unwrap();
        let canon = out.replace(' ', "");
        assert!(
            canon.contains("CredentialRef<dynBitbucketBearerPhantom"),
            "Option wrapper should rewrite inner: {out}"
        );
    }

    #[test]
    fn pattern2_inside_box_rewrites() {
        let out = rewrite_type_to_string("Box<CredentialRef<dyn BitbucketBearer>>").unwrap();
        let canon = out.replace(' ', "");
        assert!(
            canon.contains("CredentialRef<dynBitbucketBearerPhantom"),
            "Box wrapper should rewrite inner: {out}"
        );
    }

    #[test]
    fn pattern2_inside_nested_wrapper_chain_rewrites() {
        // Defense in depth — the recursive walk must reach an arbitrarily
        // nested CredentialRef. This catches any future regression that
        // would short-circuit the walk after the first non-CredentialRef
        // segment.
        let out =
            rewrite_type_to_string("Vec<Option<Box<CredentialRef<dyn BitbucketBearer>>>>").unwrap();
        let canon = out.replace(' ', "");
        assert!(
            canon.contains("CredentialRef<dynBitbucketBearerPhantom"),
            "Nested wrapper chain should rewrite inner: {out}"
        );
    }

    #[test]
    fn pattern2_wrapper_with_send_sync_marker_preservation() {
        // I4 + C1 interaction — wrapper rewrite must still preserve
        // marker bounds on the inner capability.
        let out = rewrite_type_to_string("Vec<CredentialRef<dyn BitbucketBearer + Send + Sync>>")
            .unwrap();
        let canon = out.replace(' ', "");
        assert!(
            canon.contains("BitbucketBearerPhantom"),
            "capability must be suffixed inside wrapper: {out}"
        );
        assert!(
            !canon.contains("SendPhantom"),
            "Send must not be suffixed inside wrapper: {out}"
        );
        assert!(
            !canon.contains("SyncPhantom"),
            "Sync must not be suffixed inside wrapper: {out}"
        );
    }
}
