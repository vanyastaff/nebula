//! `#[action_phantom]` attribute macro per Tech Spec 2.7 + ADR-0035 4.3.
//!
//! Rewrites struct fields whose type is `CredentialRef<dyn X>` to
//! `CredentialRef<dyn XPhantom>` in the emitted item. Pattern 1 (concrete
//! `CredentialRef<ConcreteCredential>`) is a pass-through - no `dyn`,
//! no rewrite. The attribute is silent: no diagnostic is emitted on
//! rewrite. Tech Spec 2.7 line 487 codifies "rewrites silently".
//!
//! ## Why an attribute, not a derive
//!
//! Derives may only emit *new* items; they cannot mutate the input.
//! The phantom rewrite must edit the user-written struct in place so
//! that the field type the action body sees is the dyn-compatible
//! `CredentialRef<dyn XPhantom>`. ADR-0035 amendment 2026-04-24-B
//! prescribes the phantom-shim form; Tech Spec 2.7 routes the
//! translation through `#[action_phantom]`.
//!
//! ## Why the name `action_phantom`, not `action`
//!
//! `#[derive(Action)]` declares `#[action(key = ..., ...)]` as an inert
//! helper attribute. A bare `#[action]` attribute macro alongside the
//! derive helper would put two different things called `#[action]` on
//! the same struct - confusing for readers, and a real footgun if any
//! user `use`d the macro into scope without realising. `action_phantom`
//! names what the macro does (phantom-shim rewrite) and avoids the
//! collision entirely.
//!
//! ## Coexistence with `#[derive(Action)]`
//!
//! `#[action_phantom]` is the rewrite gate; `#[derive(Action)]` is the
//! `DeclaresDependencies` + `Action` impl emitter. Authors who use
//! capability-bound dyn fields apply both - attribute first, derive
//! second.

use nebula_macro_support::credential_ref;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{ItemStruct, parse_macro_input};

/// Entry point for `#[action_phantom]`.
pub(crate) fn expand(args: TokenStream, input: TokenStream) -> TokenStream {
    let args2: TokenStream2 = args.into();
    if !args2.is_empty() {
        return syn::Error::new_spanned(
            &args2,
            "#[action_phantom] attribute does not accept arguments; remove them",
        )
        .to_compile_error()
        .into();
    }

    let mut item: ItemStruct = parse_macro_input!(input as ItemStruct);
    credential_ref::rewrite_struct_credential_refs(&mut item);
    quote!(#item).into()
}
