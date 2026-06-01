//! `#[nebula_credential::credential]` attribute macro — ADR-0088 D1.
//!
//! The single declaration site for a credential. The author writes ONE
//! inherent `impl` block carrying the associated types and the methods the
//! credential actually supports; this macro reads which methods are present
//! and emits:
//!
//! - the base [`Credential`] impl (`KEY` + `Properties`/`Scheme`/`State` +
//!   `metadata`/`project`/`resolve`),
//! - one capability sub-trait impl per capability **method** supplied
//!   (`refresh` ⇒ `Refreshable`, `revoke` ⇒ `Revocable`, `test` ⇒ `Testable`,
//!   `continue_resolve` ⇒ `Interactive`, `release` ⇒ `Dynamic`),
//! - the five `plugin_capability_report::IsX` consts (`true` exactly for the
//!   capabilities whose method is present),
//! - a [`CredentialLifecycle::policy`] whose `RefreshStrategy`/`RevokeStrategy`
//!   are derived from those same methods.
//!
//! # Why an attribute macro and not a richer `#[derive]`
//!
//! Capability is **inferred from method presence**, never declared. There is
//! no `capabilities(refreshable)` flag that could disagree with the impl — a
//! credential is `Refreshable` *iff* it has a `refresh` method, so the
//! capability-report consts and the lifecycle policy can never lie about what
//! the type implements. This preserves the `E0046` compile-gate that the
//! capability sub-trait split bought (declaring a capability you did not
//! implement is unrepresentable) while collapsing the old four declaration
//! sites (trait impl + sub-trait impl + `IsX` const + lifecycle policy) into
//! one.
//!
//! # Container arguments (`#[credential(...)]`)
//!
//! - `key = "..."` — stable credential type key (required; becomes `Credential::KEY`).
//! - `category = <CredentialCategory variant>` — the structural lifecycle kind
//!   (required; e.g. `StaticSecret`, `RefreshPair`, `Leased`). Drives the
//!   synthesized [`CredentialPolicy::category`].
//! - `name = "..."` — human-readable name (required only when no `metadata`
//!   method is supplied — used to synthesize one).
//! - `description = "..."` — catalog description (optional; defaults to `name`).
//! - `icon = "..."` — catalog icon id (optional).
//! - `doc_url = "..."` — documentation URL (optional).
//!
//! # Recognized `impl` items
//!
//! | Item | Routes to |
//! |------|-----------|
//! | `type Properties` / `type Scheme` / `type State` | `Credential` (all three required) |
//! | `type Pending` | `Interactive` (required iff `continue_resolve` present) |
//! | `fn metadata` | `Credential` (optional — synthesized from args if absent) |
//! | `fn project` / `fn resolve` | `Credential` (both required) |
//! | `fn refresh` (+ `const REFRESH_POLICY`) | `Refreshable` |
//! | `fn revoke` | `Revocable` |
//! | `fn test` | `Testable` |
//! | `fn continue_resolve` | `Interactive` |
//! | `fn release` (+ `const LEASE_TTL`) | `Dynamic` |
//! | `fn policy` | `CredentialLifecycle` (optional — synthesized if absent) |
//!
//! Any other item is rejected with a compile error, so a typo in a method name
//! cannot silently drop a capability. Inherent helpers (e.g. an OAuth2
//! `initiate_authorization_code`) live in a *separate* plain `impl` block
//! alongside the annotated one — they are not part of the credential contract.

use nebula_macro_support::{attrs, diag};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Attribute, ImplItem, ItemImpl, Visibility, parse::Parser};

/// The `CredentialCategory` variants accepted by `category = ...`. Validated
/// in-macro so a typo surfaces here with the full list rather than as an
/// opaque "no variant" error inside generated code.
const CATEGORIES: &[&str] = &[
    "StaticSecret",
    "SignedRequest",
    "BearerWithExp",
    "RefreshPair",
    "FederatedExchange",
    "InteractiveRedirect",
    "KeyPair",
    "Leased",
    "Session",
    "ConnectionString",
];

/// Entry point for `#[nebula_credential::credential(...)]`.
pub(crate) fn expand(args: TokenStream, input: TokenStream) -> TokenStream {
    match expand_inner(args.into(), input) {
        Ok(ts) => ts.into(),
        Err(e) => diag::to_compile_error(e).into(),
    }
}

/// Recognized items pulled out of the annotated `impl` block.
#[derive(Default)]
struct Items {
    // Associated types.
    properties: Option<ImplItem>,
    scheme: Option<ImplItem>,
    state: Option<ImplItem>,
    pending: Option<ImplItem>,
    // Consts.
    refresh_policy: Option<ImplItem>,
    lease_ttl: Option<ImplItem>,
    // Methods.
    metadata: Option<ImplItem>,
    project: Option<ImplItem>,
    resolve: Option<ImplItem>,
    refresh: Option<ImplItem>,
    revoke: Option<ImplItem>,
    test: Option<ImplItem>,
    continue_resolve: Option<ImplItem>,
    release: Option<ImplItem>,
    policy: Option<ImplItem>,
}

fn expand_inner(args: TokenStream2, input: TokenStream) -> syn::Result<TokenStream2> {
    let item: ItemImpl = syn::parse(input)?;

    if let Some((_, path, _)) = &item.trait_ {
        return Err(diag::error_spanned(
            path,
            "#[credential] applies to an inherent `impl Type { … }`, not a trait impl — \
             the macro generates the trait impls for you",
        ));
    }

    // Container args, parsed by reusing the shared `#[credential(...)]` grammar.
    let attr = Attribute::parse_outer
        .parse2(quote! { #[credential(#args)] })?
        .into_iter()
        .next()
        .ok_or_else(|| {
            diag::error_spanned(&item.self_ty, "missing #[credential(...)] arguments")
        })?;
    let attr_args =
        attrs::parse_attr(&attr, "credential")?.unwrap_or(attrs::AttrArgs { items: Vec::new() });

    let key = attr_args.require_string("key", &item.self_ty)?;
    let category = require_category(&attr_args, &item.self_ty)?;
    let name = attr_args.get_string("name");
    let description = attr_args.get_string("description");
    let icon = attr_args.get_string("icon");
    let doc_url = attr_args.get_string("doc_url");

    let items = classify_items(&item)?;

    // Required surface.
    let properties = items.properties.as_ref().ok_or_else(|| {
        diag::error_spanned(
            &item.self_ty,
            "#[credential] requires `type Properties = …;`",
        )
    })?;
    let scheme = items.scheme.as_ref().ok_or_else(|| {
        diag::error_spanned(&item.self_ty, "#[credential] requires `type Scheme = …;`")
    })?;
    let state = items.state.as_ref().ok_or_else(|| {
        diag::error_spanned(&item.self_ty, "#[credential] requires `type State = …;`")
    })?;
    let project = items.project.as_ref().ok_or_else(|| {
        diag::error_spanned(&item.self_ty, "#[credential] requires a `fn project(…)`")
    })?;
    let resolve = items.resolve.as_ref().ok_or_else(|| {
        diag::error_spanned(
            &item.self_ty,
            "#[credential] requires an `async fn resolve(…)`",
        )
    })?;

    // Interactive: `continue_resolve` and `type Pending` come as a pair.
    match (&items.continue_resolve, &items.pending) {
        (Some(_), None) => {
            return Err(diag::error_spanned(
                &item.self_ty,
                "`fn continue_resolve` requires a `type Pending = …;` (the Interactive \
                 capability needs its typed pending state)",
            ));
        },
        (None, Some(p)) => {
            return Err(diag::error_spanned(
                p,
                "`type Pending` is only valid alongside a `fn continue_resolve` — \
                 remove it or add the interactive continuation method",
            ));
        },
        _ => {},
    }

    // Stray tuning consts without their capability method.
    if let Some(rp) = &items.refresh_policy
        && items.refresh.is_none()
    {
        return Err(diag::error_spanned(
            rp,
            "`const REFRESH_POLICY` is only valid alongside a `fn refresh` (Refreshable)",
        ));
    }
    if let Some(ttl) = &items.lease_ttl
        && items.release.is_none()
    {
        return Err(diag::error_spanned(
            ttl,
            "`const LEASE_TTL` is only valid alongside a `fn release` (Dynamic)",
        ));
    }

    // A leased/dynamic credential's expiry and renewability live in its state
    // (lease id + duration), which the macro cannot read. A synthesized
    // `RefreshStrategy::Lease` policy would therefore carry `lease: None` and
    // report the credential as non-expiring / non-renewable — a lie for a
    // Vault/k8s-style secret. Require an explicit `fn policy` in that case.
    if items.release.is_some() && items.policy.is_none() {
        return Err(diag::error_spanned(
            &item.self_ty,
            "a credential with `fn release` (Dynamic/leased) must hand-write `fn policy` — \
             the macro cannot infer the lease id/duration from state, so it cannot synthesize \
             a correct `RefreshStrategy::Lease` policy (set `lease: Some(LeaseRef { … })`)",
        ));
    }

    let self_ty = &item.self_ty;
    let (impl_generics, _ty_generics, where_clause) = item.generics.split_for_impl();

    // Forward the author's outer attributes (e.g. `#[cfg(...)]`,
    // `#[allow(...)]`) — minus doc comments — onto every generated impl, so a
    // cfg-gated or lint-configured annotated block yields cfg-gated /
    // lint-configured trait impls rather than silently unconditional ones.
    let outer_attrs: Vec<&Attribute> = item
        .attrs
        .iter()
        .filter(|a| !a.path().is_ident("doc"))
        .collect();
    let fwd = quote! { #(#outer_attrs)* };

    // ── metadata: relocate the author's, or synthesize from args ──────────
    let metadata_fn = if let Some(m) = &items.metadata {
        let m = trait_item(m.clone());
        quote! { #m }
    } else {
        let name = name.as_deref().ok_or_else(|| {
            diag::error_spanned(
                self_ty,
                "#[credential] needs either a `fn metadata(…)` or a `name = \"…\"` \
                 argument to synthesize one",
            )
        })?;
        let description = description.as_deref().unwrap_or(name);
        let scheme_ty = assoc_type(scheme)?;
        if icon.is_none() && doc_url.is_none() {
            // No icon / doc_url ⇒ the infallible `for_credential` constructor,
            // so the common synthesized case emits no `expect(...)` into
            // library code. (`credential_key!` is re-exported from
            // `nebula_credential`, so a consumer that does not directly depend
            // on `nebula-core` still resolves the path.)
            quote! {
                fn metadata() -> ::nebula_credential::CredentialMetadata
                where
                    Self: Sized,
                {
                    ::nebula_credential::CredentialMetadata::for_credential::<Self>(
                        ::nebula_credential::credential_key!(#key),
                        #name,
                        #description,
                        <#scheme_ty as ::nebula_credential::AuthScheme>::pattern(),
                    )
                }
            }
        } else {
            // Icon / doc_url require the builder, whose `build()` is fallible;
            // the `expect` here mirrors the hand-written built-ins and the
            // legacy `#[derive(Credential)]` (the `metadata()` trait method is
            // infallible, and the builder is the only icon/doc_url path).
            let mut builder = quote! {
                ::nebula_credential::CredentialMetadata::builder()
                    .key(::nebula_credential::credential_key!(#key))
                    .name(#name)
                    .description(#description)
                    .schema(::nebula_credential::schema_of::<Self::Properties>())
                    .pattern(<#scheme_ty as ::nebula_credential::AuthScheme>::pattern())
            };
            if let Some(icon) = &icon {
                builder = quote! { #builder .icon(#icon) };
            }
            if let Some(url) = &doc_url {
                builder = quote! { #builder .documentation_url(#url) };
            }
            quote! {
                fn metadata() -> ::nebula_credential::CredentialMetadata
                where
                    Self: Sized,
                {
                    #builder .build().expect("credential metadata is valid")
                }
            }
        }
    };

    let properties_ty = trait_item(properties.clone());
    let scheme_item = trait_item(scheme.clone());
    let state_item = trait_item(state.clone());
    let project_fn = trait_item(project.clone());
    let resolve_fn = trait_item(resolve.clone());

    let credential_impl = quote! {
        #fwd
        impl #impl_generics ::nebula_credential::Credential for #self_ty #where_clause {
            #properties_ty
            #scheme_item
            #state_item

            const KEY: &'static str = #key;

            #metadata_fn
            #project_fn
            #resolve_fn
        }
    };

    // ── capability sub-trait impls (presence-gated) ───────────────────────
    let refreshable_impl = items.refresh.as_ref().map(|refresh| {
        let refresh_policy = items.refresh_policy.clone().map(trait_item);
        let refresh = trait_item(refresh.clone());
        quote! {
            #fwd
            impl #impl_generics ::nebula_credential::Refreshable for #self_ty #where_clause {
                #refresh_policy
                #refresh
            }
        }
    });
    let revocable_impl = items.revoke.as_ref().map(|revoke| {
        let revoke = trait_item(revoke.clone());
        quote! {
            #fwd
            impl #impl_generics ::nebula_credential::Revocable for #self_ty #where_clause {
                #revoke
            }
        }
    });
    let testable_impl = items.test.as_ref().map(|test| {
        let test = trait_item(test.clone());
        quote! {
            #fwd
            impl #impl_generics ::nebula_credential::Testable for #self_ty #where_clause {
                #test
            }
        }
    });
    let interactive_impl = items.continue_resolve.as_ref().map(|cont| {
        let pending = items.pending.clone().map(trait_item);
        let cont = trait_item(cont.clone());
        quote! {
            #fwd
            impl #impl_generics ::nebula_credential::Interactive for #self_ty #where_clause {
                #pending
                #cont
            }
        }
    });
    let dynamic_impl = items.release.as_ref().map(|release| {
        let lease_ttl = items.lease_ttl.clone().map(trait_item);
        let release = trait_item(release.clone());
        quote! {
            #fwd
            impl #impl_generics ::nebula_credential::Dynamic for #self_ty #where_clause {
                #lease_ttl
                #release
            }
        }
    });

    // ── capability report consts ──────────────────────────────────────────
    let is_interactive = items.continue_resolve.is_some();
    let is_refreshable = items.refresh.is_some();
    let is_revocable = items.revoke.is_some();
    let is_testable = items.test.is_some();
    let is_dynamic = items.release.is_some();
    let capability_impls = quote! {
        #fwd
        impl #impl_generics
            ::nebula_credential::contract::plugin_capability_report::IsInteractive
            for #self_ty #where_clause
        { const VALUE: bool = #is_interactive; }
        #fwd
        impl #impl_generics
            ::nebula_credential::contract::plugin_capability_report::IsRefreshable
            for #self_ty #where_clause
        { const VALUE: bool = #is_refreshable; }
        #fwd
        impl #impl_generics
            ::nebula_credential::contract::plugin_capability_report::IsRevocable
            for #self_ty #where_clause
        { const VALUE: bool = #is_revocable; }
        #fwd
        impl #impl_generics
            ::nebula_credential::contract::plugin_capability_report::IsTestable
            for #self_ty #where_clause
        { const VALUE: bool = #is_testable; }
        #fwd
        impl #impl_generics
            ::nebula_credential::contract::plugin_capability_report::IsDynamic
            for #self_ty #where_clause
        { const VALUE: bool = #is_dynamic; }
    };

    // ── lifecycle policy: relocate the author's, or synthesize ────────────
    let lifecycle_impl = if let Some(policy) = &items.policy {
        let policy = trait_item(policy.clone());
        quote! {
            #fwd
            impl #impl_generics ::nebula_credential::CredentialLifecycle
                for #self_ty #where_clause
            {
                #policy
            }
        }
    } else {
        let category_ident = &category;
        let refresh_strategy = if is_refreshable {
            quote! { ::nebula_credential::RefreshStrategy::RefreshToken }
        } else if is_dynamic {
            quote! { ::nebula_credential::RefreshStrategy::Lease }
        } else {
            quote! { ::nebula_credential::RefreshStrategy::Static }
        };
        let revoke_strategy = if is_revocable {
            quote! { ::nebula_credential::RevokeStrategy::HandleBased }
        } else {
            quote! { ::nebula_credential::RevokeStrategy::None }
        };
        quote! {
            #fwd
            impl #impl_generics ::nebula_credential::CredentialLifecycle
                for #self_ty #where_clause
            {
                fn policy(_state: &Self::State) -> ::nebula_credential::CredentialPolicy
                where
                    Self: Sized,
                {
                    ::nebula_credential::CredentialPolicy {
                        category: ::nebula_credential::CredentialCategory::#category_ident,
                        expires_at: ::core::option::Option::None,
                        lease: ::core::option::Option::None,
                        refresh: #refresh_strategy,
                        revoke: #revoke_strategy,
                    }
                }
            }
        }
    };

    Ok(quote! {
        #credential_impl
        #refreshable_impl
        #revocable_impl
        #testable_impl
        #interactive_impl
        #dynamic_impl
        #capability_impls
        #lifecycle_impl
    })
}

/// Pull `category = <Ident>` out of the args, validating the variant name.
fn require_category(
    attr_args: &attrs::AttrArgs,
    span: &impl quote::ToTokens,
) -> syn::Result<syn::Ident> {
    let ident = attr_args.get_ident("category").cloned().ok_or_else(|| {
        diag::error_spanned(
            span,
            "#[credential] requires `category = <CredentialCategory variant>` \
             (e.g. `StaticSecret`, `RefreshPair`, `Leased`)",
        )
    })?;
    let name = ident.to_string();
    if !CATEGORIES.contains(&name.as_str()) {
        return Err(diag::error_spanned(
            &ident,
            format!(
                "unknown credential category `{name}` — expected one of: {}",
                CATEGORIES.join(", ")
            ),
        ));
    }
    Ok(ident)
}

/// Classify each `impl` item by name into the recognized buckets, rejecting
/// anything unrecognized so a misspelled method cannot silently drop a
/// capability.
fn classify_items(item: &ItemImpl) -> syn::Result<Items> {
    let mut out = Items::default();
    for it in &item.items {
        match it {
            ImplItem::Type(ty) => {
                let slot = match ty.ident.to_string().as_str() {
                    "Properties" => &mut out.properties,
                    "Scheme" => &mut out.scheme,
                    "State" => &mut out.state,
                    "Pending" => &mut out.pending,
                    other => {
                        return Err(unknown_item(&ty.ident, other, "associated type"));
                    },
                };
                set_once(slot, it.clone(), &ty.ident)?;
            },
            ImplItem::Const(c) => {
                let slot = match c.ident.to_string().as_str() {
                    "REFRESH_POLICY" => &mut out.refresh_policy,
                    "LEASE_TTL" => &mut out.lease_ttl,
                    "KEY" => {
                        return Err(diag::error_spanned(
                            &c.ident,
                            "`KEY` is supplied via `#[credential(key = \"…\")]`, not as a \
                             `const` in the impl block",
                        ));
                    },
                    other => return Err(unknown_item(&c.ident, other, "const")),
                };
                set_once(slot, it.clone(), &c.ident)?;
            },
            ImplItem::Fn(f) => {
                let ident = &f.sig.ident;
                let slot = match ident.to_string().as_str() {
                    "metadata" => &mut out.metadata,
                    "project" => &mut out.project,
                    "resolve" => &mut out.resolve,
                    "refresh" => &mut out.refresh,
                    "revoke" => &mut out.revoke,
                    "test" => &mut out.test,
                    "continue_resolve" => &mut out.continue_resolve,
                    "release" => &mut out.release,
                    "policy" => &mut out.policy,
                    other => {
                        return Err(diag::error_spanned(
                            ident,
                            format!(
                                "unrecognized method `{other}` in #[credential] impl — move \
                                 inherent helpers to a separate `impl` block. Recognized \
                                 methods: metadata, project, resolve, refresh, revoke, test, \
                                 continue_resolve, release, policy"
                            ),
                        ));
                    },
                };
                set_once(slot, it.clone(), ident)?;
            },
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "unsupported item in #[credential] impl — only associated types, \
                     capability consts, and recognized methods are allowed",
                ));
            },
        }
    }
    Ok(out)
}

fn unknown_item(span: &impl quote::ToTokens, name: &str, kind: &str) -> syn::Error {
    diag::error_spanned(
        span,
        format!("unrecognized {kind} `{name}` in #[credential] impl"),
    )
}

fn set_once(
    slot: &mut Option<ImplItem>,
    value: ImplItem,
    span: &impl quote::ToTokens,
) -> syn::Result<()> {
    if slot.is_some() {
        return Err(diag::error_spanned(
            span,
            "duplicate item in #[credential] impl",
        ));
    }
    *slot = Some(value);
    Ok(())
}

/// Force a relocated item to inherited visibility — trait-impl items cannot
/// carry `pub`, but an author writing what looks like an inherent block might.
fn trait_item(mut it: ImplItem) -> ImplItem {
    match &mut it {
        ImplItem::Fn(f) => f.vis = Visibility::Inherited,
        ImplItem::Type(t) => t.vis = Visibility::Inherited,
        ImplItem::Const(c) => c.vis = Visibility::Inherited,
        _ => {},
    }
    it
}

/// Extract the right-hand-side type of an `ImplItem::Type` (the `X` in
/// `type Scheme = X;`) for use in generated metadata bounds.
///
/// `classify_items` only ever routes `type Scheme` here, so the non-type case
/// is a macro-internal invariant violation — surfaced as a normal `syn::Error`
/// (a proper diagnostic) rather than a proc-macro panic.
fn assoc_type(it: &ImplItem) -> syn::Result<&syn::Type> {
    match it {
        ImplItem::Type(t) => Ok(&t.ty),
        other => Err(diag::error_spanned(
            other,
            "internal error: `type Scheme` was not classified as an associated type — \
             please report this as a `#[credential]` macro bug",
        )),
    }
}
