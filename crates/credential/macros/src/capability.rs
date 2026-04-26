//! `#[capability]` attribute macro per ADR-0035 4 + Tech Spec 2.6.
//!
//! Expands a single capability trait declaration into the full ADR-0035
//! canonical form: real trait, service/scheme blanket impl,
//! sealed-blanket, phantom trait, and phantom blanket. Hides the
//! two-trait verbosity from everyday plugin and built-in code.
//!
//! ## Input
//!
//! ```ignore
//! #[capability(scheme_bound = AcceptsBearer, sealed = BearerSealed)]
//! pub trait BitbucketBearer: BitbucketCredential {}
//! ```
//!
//! ## Output (hand-expanded equivalent)
//!
//! ```ignore
//! pub trait BitbucketBearer: BitbucketCredential {}
//!
//! impl<T> BitbucketBearer for T
//! where
//!     T: BitbucketCredential,
//!     <T as ::nebula_credential::Credential>::Scheme: AcceptsBearer,
//! {}
//!
//! impl<T: BitbucketBearer> sealed_caps::BearerSealed for T {}
//!
//! pub trait BitbucketBearerPhantom:
//!     sealed_caps::BearerSealed
//!     + ::core::marker::Send
//!     + ::core::marker::Sync
//! {}
//!
//! impl<T: BitbucketBearer> BitbucketBearerPhantom for T {}
//! ```
//!
//! ## Caller obligations
//!
//! Per ADR-0035 4.1 / 4.2, the macro does NOT emit `mod sealed_caps`.
//! Crate authors declare it manually at crate root with one inner trait
//! per capability. A missing module produces `E0433` at the emitted
//! sealed-blanket impl line, with the standard "unresolved module"
//! rustc diagnostic. Tech Spec 2.6 documents the onboarding step.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Ident, ItemTrait, Path, Token, TypeParamBound,
    parse::{Parse, ParseStream},
    parse_macro_input,
    spanned::Spanned,
};

/// Parsed `#[capability(...)]` arguments.
struct CapabilityArgs {
    scheme_bound: Path,
    sealed: Ident,
}

impl Parse for CapabilityArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut scheme_bound: Option<Path> = None;
        let mut sealed: Option<Ident> = None;

        if input.is_empty() {
            return Err(syn::Error::new(
                input.span(),
                "#[capability] requires `scheme_bound = <Path>, sealed = <Ident>` arguments",
            ));
        }

        loop {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            if key == "scheme_bound" {
                if scheme_bound.is_some() {
                    return Err(syn::Error::new(
                        key.span(),
                        "duplicate `scheme_bound` argument in #[capability]",
                    ));
                }
                scheme_bound = Some(input.parse::<Path>()?);
            } else if key == "sealed" {
                if sealed.is_some() {
                    return Err(syn::Error::new(
                        key.span(),
                        "duplicate `sealed` argument in #[capability]",
                    ));
                }
                sealed = Some(input.parse::<Ident>()?);
            } else {
                return Err(syn::Error::new(
                    key.span(),
                    format!(
                        "unknown #[capability] argument `{key}`; \
                         expected `scheme_bound` or `sealed`"
                    ),
                ));
            }

            if input.is_empty() {
                break;
            }
            input.parse::<Token![,]>()?;
            if input.is_empty() {
                break;
            }
        }

        let scheme_bound = scheme_bound.ok_or_else(|| {
            syn::Error::new(
                input.span(),
                "#[capability] missing required argument `scheme_bound = <Path>`",
            )
        })?;
        let sealed = sealed.ok_or_else(|| {
            syn::Error::new(
                input.span(),
                "#[capability] missing required argument `sealed = <Ident>`",
            )
        })?;

        Ok(Self {
            scheme_bound,
            sealed,
        })
    }
}

/// Entry point for `#[capability]`.
pub(crate) fn expand(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as CapabilityArgs);
    let trait_def = parse_macro_input!(input as ItemTrait);

    match expand_inner(args, trait_def) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand_inner(args: CapabilityArgs, trait_def: ItemTrait) -> syn::Result<TokenStream2> {
    if !trait_def.items.is_empty() {
        return Err(syn::Error::new(
            trait_def.brace_token.span.span(),
            "#[capability] trait body must be empty - capability traits are markers, not method carriers (semantics enforced via service trait + scheme blanket)",
        ));
    }

    let real_name = trait_def.ident.clone();
    let phantom_name = Ident::new(&format!("{real_name}Phantom"), real_name.span());
    let scheme_bound = &args.scheme_bound;
    let sealed = &args.sealed;
    let vis = &trait_def.vis;
    let service_supertrait = extract_service_supertrait(&trait_def)?;

    let expanded = quote! {
        #trait_def

        impl<__CapabilityT> #real_name for __CapabilityT
        where
            __CapabilityT: #service_supertrait,
            <__CapabilityT as ::nebula_credential::Credential>::Scheme: #scheme_bound,
        {}

        impl<__CapabilityT: #real_name> sealed_caps::#sealed for __CapabilityT {}

        #vis trait #phantom_name:
            sealed_caps::#sealed
            + ::core::marker::Send
            + ::core::marker::Sync
        {}

        impl<__CapabilityT: #real_name> #phantom_name for __CapabilityT {}
    };

    Ok(expanded)
}

/// Extract the service supertrait path from `trait X: Service {}`.
fn extract_service_supertrait(t: &ItemTrait) -> syn::Result<Path> {
    let mut candidates: Vec<&Path> = Vec::new();

    for bound in &t.supertraits {
        match bound {
            TypeParamBound::Trait(trait_bound) => {
                if is_marker_bound(&trait_bound.path) {
                    continue;
                }
                candidates.push(&trait_bound.path);
            },
            TypeParamBound::Lifetime(_) => continue,
            _ => continue,
        }
    }

    match candidates.len() {
        0 => Err(syn::Error::new(
            t.ident.span(),
            "#[capability] requires exactly one service-trait supertrait (e.g. `pub trait BitbucketBearer: BitbucketCredential {}`); none was found",
        )),
        1 => Ok(candidates[0].clone()),
        _ => Err(syn::Error::new(
            candidates[1].span(),
            "#[capability] requires exactly one service-trait supertrait (e.g. `pub trait BitbucketBearer: BitbucketCredential {}`); multiple non-marker bounds found",
        )),
    }
}

fn is_marker_bound(path: &Path) -> bool {
    let Some(last) = path.segments.last() else {
        return false;
    };
    matches!(
        last.ident.to_string().as_str(),
        "Send" | "Sync" | "Sized" | "Unpin" | "Copy" | "Clone"
    )
}
