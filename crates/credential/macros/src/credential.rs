//! `#[derive(Credential)]` macro implementation — Phase 5 / ADR-0044 / M6 redesign.
//!
//! Mirrors `#[derive(Resource)]` (Phase 4) and `#[derive(Action)]` (Phase 3) on
//! the slot-binding family. Credentials are the *leaf* of the dependency
//! graph (they don't depend on other credentials per spec 23) and so the
//! macro's surface is simpler than the Resource / Action variants — most
//! of the work is metadata + type-aliasing.
//!
//! # Container attribute
//!
//! `#[credential(key, name, scheme, properties|protocol, ...)]`
//!
//! - `key = "..."` — Unique credential type key (required).
//! - `name = "..."` — Human-readable name (required).
//! - `scheme = TypePath` — Auth scheme produced by `Credential::project` (required). Also doubles
//!   as the storage `State` (identity-state pattern).
//! - `properties = TypePath` — Direct path to the `<Name>Properties` struct that owns the
//!   setup-form schema. Mutually exclusive with `protocol`. The macro emits `type Properties =
//!   #properties` and reuses `<#properties as HasSchema>::schema()` through the default
//!   [`Credential::properties_schema`](nebula_credential::Credential::properties_schema) impl. When
//!   `properties` is supplied, the user is responsible for implementing [`Credential::resolve`]
//!   (and [`Credential::project`] when scheme ≠ state) on a separate inherent impl block.
//! - `protocol = TypePath` — Reusable [`StaticProtocol`](nebula_credential::StaticProtocol) for
//!   static (non-interactive) credentials. Mutually exclusive with `properties`. The macro emits
//!   `type Properties = <protocol as StaticProtocol>::Properties` and a `resolve` body that
//!   delegates to `<protocol as StaticProtocol>::build(values)`.
//! - `icon = "..."` — Catalog icon identifier (optional).
//! - `doc_url = "..."` — Documentation URL (optional).
//! - `capabilities(...)` — Sub-traits the credential implements; accepts `interactive`,
//!   `refreshable`, `revocable`, `testable`, `dynamic`. The macro emits one
//!   `plugin_capability_report::IsX` impl per capability and a parity assertion that consumes the
//!   actual sub-trait bound, so a missing `impl Refreshable for X` fails to compile.
//!
//! # Outer attributes (struct-level helpers)
//!
//! - `#[uses_resource(TypeName, purpose = "...")]` — Declare a resource dependency (repeatable).
//! - `#[uses_credential(...)]` — Forbidden (spec 23): credential-to-credential static dependencies
//!   are not allowed; use `ctx.credential::<C>()` for runtime composition. Emits a compile error.
//!
//! # Phase 5 changes
//!
//! - Renamed emitted `type Input` → `type Properties` to mirror `Action::Input` /
//!   `Resource::Config` and shift schema ownership from instance metadata to a type-level companion
//!   struct. The default `Credential::properties_schema()` body reads the schema via
//!   `<Self::Properties as HasSchema>::schema()`.
//! - Added a `properties = TypePath` attribute as the canonical, direct way to specify the
//!   companion struct (no longer requires plumbing through a `StaticProtocol` indirection when the
//!   user only wants the schema bridge — `protocol` is retained for the canonical static-resolve
//!   delegation pattern).
//!
//! # Examples
//!
//! Properties-mode (manual `resolve`):
//! ```ignore
//! use nebula_credential::Credential;
//! use nebula_schema::Schema;
//! use serde::Deserialize;
//!
//! #[derive(Schema, Deserialize)]
//! pub struct GithubOAuthProperties {
//!     #[field(label = "Client ID")]
//!     #[validate(required)]
//!     pub client_id: String,
//! }
//!
//! #[derive(Credential)]
//! #[credential(
//!     key = "github_oauth",
//!     name = "GitHub OAuth",
//!     scheme = OAuth2Token,
//!     properties = GithubOAuthProperties,
//!     icon = "github",
//! )]
//! pub struct GithubOAuthCredential;
//!
//! // The user provides a separate inherent impl with `project` / `resolve`
//! // when `properties` is supplied, since the macro cannot synthesize them.
//! ```
//!
//! Protocol-mode (auto-emitted `resolve`):
//! ```ignore
//! #[derive(Credential)]
//! #[credential(
//!     key = "postgres",
//!     name = "PostgreSQL",
//!     scheme = ConnectionUri,
//!     protocol = PostgresProtocol,
//!     icon = "postgres",
//! )]
//! pub struct PostgresCredential;
//! ```

use nebula_macro_support::{
    attrs::{self, AttrItem, AttrValue},
    diag,
};
use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

/// Entry point for `#[derive(Credential)]`.
pub(crate) fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand(input) {
        Ok(ts) => ts.into(),
        Err(e) => diag::to_compile_error(e).into(),
    }
}

/// Capability set declared via `#[credential(capabilities(...))]`. The
/// macro emits one `plugin_capability_report::IsX` impl per flag — `true`
/// when the flag was listed, `false` otherwise. Per Tech Spec §15.8 the
/// declaration is opt-in (default static / no capabilities) so plugin
/// authors cannot accidentally over-attest.
#[derive(Debug, Clone, Copy, Default)]
struct DeclaredCapabilities {
    interactive: bool,
    refreshable: bool,
    revocable: bool,
    testable: bool,
    dynamic: bool,
}

/// Source of `Self::Properties`:
/// - `Direct(Type)` — supplied via `#[credential(properties = TypePath)]`
/// - `ViaProtocol(Type)` — taken from `<protocol as StaticProtocol>::Properties`
enum PropertiesSource {
    Direct(syn::Type),
    ViaProtocol(syn::Type),
}

/// Parsed `#[credential(...)]` attributes.
struct CredentialAttrs {
    key: String,
    name: String,
    scheme: syn::Type,
    properties_source: PropertiesSource,
    icon: Option<String>,
    doc_url: Option<String>,
    capabilities: DeclaredCapabilities,
}

fn parse_credential_attrs(
    attr_args: &attrs::AttrArgs,
    struct_name: &syn::Ident,
) -> syn::Result<CredentialAttrs> {
    const ALLOWED: &[&str] = &[
        "key",
        "name",
        "scheme",
        "properties",
        "protocol",
        "icon",
        "doc_url",
        "capabilities",
    ];
    for item in &attr_args.items {
        let key = match item {
            AttrItem::KeyValue { key, .. } | AttrItem::Flag(key) | AttrItem::List { key, .. } => {
                key
            },
        };
        if !ALLOWED.iter().any(|allowed| key == allowed) {
            return Err(syn::Error::new_spanned(
                key,
                format!(
                    "unknown attribute `{key}` in #[credential(...)] \
                     — allowed keys: {}",
                    ALLOWED.join(", "),
                ),
            ));
        }
    }

    let key = attr_args.require_string("key", struct_name)?;
    let name = attr_args.require_string("name", struct_name)?;

    let scheme = attr_args.get_type("scheme")?.ok_or_else(|| {
        diag::error_spanned(
            struct_name,
            "#[derive(Credential)] requires `scheme = Type` attribute",
        )
    })?;

    let properties = attr_args.get_type("properties")?;
    let protocol = attr_args.get_type("protocol")?;

    let properties_source = match (properties, protocol) {
        (Some(_), Some(_)) => {
            return Err(diag::error_spanned(
                struct_name,
                "#[derive(Credential)] cannot mix `properties = ...` and `protocol = ...` — \
                 supply exactly one (use `properties` for direct schema-only bridging, \
                 `protocol` for the canonical static-resolve delegation)",
            ));
        },
        (Some(p), None) => PropertiesSource::Direct(p),
        (None, Some(p)) => PropertiesSource::ViaProtocol(p),
        (None, None) => {
            return Err(diag::error_spanned(
                struct_name,
                "#[derive(Credential)] requires either `properties = TypePath` (direct) \
                 or `protocol = TypePath` (StaticProtocol delegation) — neither was supplied",
            ));
        },
    };

    let icon = attr_args.get_string("icon");
    let doc_url = attr_args.get_string("doc_url");

    let capabilities = parse_capabilities(attr_args)?;

    Ok(CredentialAttrs {
        key,
        name,
        scheme,
        properties_source,
        icon,
        doc_url,
        capabilities,
    })
}

/// Parse the `capabilities(interactive, refreshable, revocable, testable,
/// dynamic)` list inside `#[credential(...)]`. Per Tech Spec §15.8 each
/// listed identifier flips the matching `plugin_capability_report::IsX::VALUE`
/// to `true`; unlisted flags emit `false`. Unknown identifiers and
/// non-ident values surface as compile errors so a typo cannot silently
/// suppress a capability flag.
///
/// **Duplicate handling.** Multiple `capabilities(...)` lists in the same
/// `#[credential(...)]` are rejected with a "duplicate list" error
/// (silent first-wins would discard the second author's intent).
/// A duplicate identifier inside a single list (e.g.
/// `capabilities(refreshable, refreshable)`) is likewise rejected — the
/// declaration surface is opt-in and any redundancy is more likely a
/// typo than an intent.
fn parse_capabilities(attr_args: &attrs::AttrArgs) -> syn::Result<DeclaredCapabilities> {
    let mut declared = DeclaredCapabilities::default();

    // Collect every `capabilities(...)` list. PR #582 review (CodeRabbit)
    // — silent first-wins on duplicate lists discards the second
    // author's intent; emit a span-attached error instead.
    let lists: Vec<(&syn::Ident, &Vec<AttrValue>)> = attr_args
        .items
        .iter()
        .filter_map(|item| match item {
            AttrItem::List { key, values } if key == "capabilities" => Some((key, values)),
            _ => None,
        })
        .collect();

    if lists.len() > 1 {
        // Span the second occurrence so the diagnostic points at the
        // redundant list, not the first (legitimate) one.
        let (second_key, _) = lists[1];
        return Err(diag::error_spanned(
            second_key,
            "duplicate `capabilities(...)` list inside `#[credential(...)]` — \
             declare all flags in a single list",
        ));
    }

    let Some((_, values)) = lists.into_iter().next() else {
        return Ok(declared);
    };

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for value in values {
        let ident = match value {
            AttrValue::Ident(i) => i,
            AttrValue::Lit(lit) => {
                return Err(diag::error_spanned(
                    lit,
                    "capabilities(...) accepts only bare identifiers \
                     (interactive, refreshable, revocable, testable, dynamic)",
                ));
            },
            AttrValue::Tokens(tokens) => {
                return Err(diag::error_spanned(
                    tokens,
                    "capabilities(...) accepts only bare identifiers \
                     (interactive, refreshable, revocable, testable, dynamic)",
                ));
            },
        };
        let name = ident.to_string();
        if !seen.insert(name.clone()) {
            return Err(diag::error_spanned(
                ident,
                format!(
                    "duplicate capability `{name}` in `capabilities(...)` — \
                     each capability flag must appear at most once"
                ),
            ));
        }
        match name.as_str() {
            "interactive" => declared.interactive = true,
            "refreshable" => declared.refreshable = true,
            "revocable" => declared.revocable = true,
            "testable" => declared.testable = true,
            "dynamic" => declared.dynamic = true,
            other => {
                return Err(diag::error_spanned(
                    ident,
                    format!(
                        "unknown capability `{other}` (expected one of: \
                         interactive, refreshable, revocable, testable, dynamic)"
                    ),
                ));
            },
        }
    }

    Ok(declared)
}

/// Parsed `#[uses_resource(TypeName, purpose = "...")]` attribute.
struct ResourceDep {
    type_ident: syn::Ident,
    purpose: Option<String>,
}

/// Parse all `#[uses_resource(...)]` attributes from the input.
fn parse_resource_deps(attrs: &[syn::Attribute]) -> syn::Result<Vec<ResourceDep>> {
    let mut deps = Vec::new();
    for attr in attrs {
        if let Some(args) = attrs::parse_attr(attr, "uses_resource")? {
            let type_ident = args
                .items
                .iter()
                .find_map(|item| match item {
                    AttrItem::Flag(ident) => Some(ident.clone()),
                    _ => None,
                })
                .ok_or_else(|| {
                    diag::error_spanned(
                        attr,
                        "#[uses_resource(TypeName)] requires a type name as the first argument",
                    )
                })?;
            let purpose = args.get_string("purpose");
            deps.push(ResourceDep {
                type_ident,
                purpose,
            });
        }
    }
    Ok(deps)
}

/// Check for forbidden `#[uses_credential(...)]` attributes.
fn check_uses_credential(attrs: &[syn::Attribute]) -> syn::Result<()> {
    for attr in attrs {
        if attr.path().is_ident("uses_credential") {
            return Err(diag::error_spanned(
                attr,
                "credential-to-credential static dependencies are forbidden (spec 23). \
                 Use ctx.credential::<C>() for runtime composition.",
            ));
        }
    }
    Ok(())
}

/// Convert a PascalCase identifier to snake_case for use as a resource key.
fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(ch.to_ascii_lowercase());
        } else {
            result.push(ch);
        }
    }
    result
}

fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    match &input.data {
        Data::Struct(data) => {
            if !matches!(&data.fields, Fields::Unit) {
                return Err(syn::Error::new(
                    input.ident.span(),
                    "#[derive(Credential)] requires a unit struct (e.g. `struct MyCredential;`)",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new(
                input.ident.span(),
                "#[derive(Credential)] can only be used on structs",
            ));
        },
    }

    // Reject forbidden uses_credential attribute.
    check_uses_credential(&input.attrs)?;

    let resource_deps = parse_resource_deps(&input.attrs)?;

    let attr_args = attrs::parse_attrs(&input.attrs, "credential")?;
    let attrs = parse_credential_attrs(&attr_args, struct_name)?;

    let key = &attrs.key;
    let name = &attrs.name;
    let scheme = &attrs.scheme;
    let caps = attrs.capabilities;

    // Generate DeclaresDependencies impl.
    let deps_impl = if resource_deps.is_empty() {
        quote! {
            impl #impl_generics ::nebula_core::DeclaresDependencies
                for #struct_name #ty_generics #where_clause
            {}
        }
    } else {
        let resource_stmts = resource_deps.iter().map(|dep| {
            let ty = &dep.type_ident;
            let key_str = to_snake_case(&ty.to_string());
            let type_name_str = ty.to_string();
            if let Some(purpose) = &dep.purpose {
                quote! {
                    .resource(
                        ::nebula_core::ResourceRequirement::new(
                            #key_str,
                            ::std::any::TypeId::of::<#ty>(),
                            #type_name_str,
                        ).purpose(#purpose)
                    )
                }
            } else {
                quote! {
                    .resource(
                        ::nebula_core::ResourceRequirement::new(
                            #key_str,
                            ::std::any::TypeId::of::<#ty>(),
                            #type_name_str,
                        )
                    )
                }
            }
        });
        quote! {
            impl #impl_generics ::nebula_core::DeclaresDependencies
                for #struct_name #ty_generics #where_clause
            {
                fn dependencies() -> ::nebula_core::Dependencies
                where
                    Self: Sized,
                {
                    ::nebula_core::Dependencies::new()
                        #(#resource_stmts)*
                }
            }
        }
    };

    // Build the metadata body: use builder when icon/doc_url are set,
    // otherwise use the simpler `for_credential` constructor.
    let metadata_body = {
        let has_extras = attrs.icon.is_some() || attrs.doc_url.is_some();
        if has_extras {
            let mut builder_chain = quote! {
                ::nebula_credential::CredentialMetadata::builder()
                    .key(::nebula_core::credential_key!(#key))
                    .name(#name)
                    .description(#name)
                    .schema(Self::properties_schema())
                    .pattern(<#scheme as ::nebula_credential::AuthScheme>::pattern())
            };
            if let Some(icon) = &attrs.icon {
                builder_chain = quote! { #builder_chain .icon(#icon) };
            }
            if let Some(url) = &attrs.doc_url {
                builder_chain = quote! { #builder_chain .documentation_url(#url) };
            }
            quote! { #builder_chain .build().expect("credential metadata is valid") }
        } else {
            quote! {
                ::nebula_credential::CredentialMetadata::for_credential::<Self>(
                    ::nebula_core::credential_key!(#key),
                    #name,
                    #name,
                    <#scheme as ::nebula_credential::AuthScheme>::pattern(),
                )
            }
        }
    };

    // Per Tech Spec §15.8 (closes security-lead N6) emit one
    // `plugin_capability_report::IsX` impl per capability flag — `true`
    // when the flag was listed in `capabilities(...)`, `false` otherwise.
    // The five impls together satisfy the bound on
    // `CredentialRegistry::register` and make capability discovery
    // type-driven rather than self-attested.
    let interactive_value = caps.interactive;
    let refreshable_value = caps.refreshable;
    let revocable_value = caps.revocable;
    let testable_value = caps.testable;
    let dynamic_value = caps.dynamic;

    let capability_impls = quote! {
        impl #impl_generics
            ::nebula_credential::contract::plugin_capability_report::IsInteractive
            for #struct_name #ty_generics #where_clause
        {
            const VALUE: bool = #interactive_value;
        }
        impl #impl_generics
            ::nebula_credential::contract::plugin_capability_report::IsRefreshable
            for #struct_name #ty_generics #where_clause
        {
            const VALUE: bool = #refreshable_value;
        }
        impl #impl_generics
            ::nebula_credential::contract::plugin_capability_report::IsRevocable
            for #struct_name #ty_generics #where_clause
        {
            const VALUE: bool = #revocable_value;
        }
        impl #impl_generics
            ::nebula_credential::contract::plugin_capability_report::IsTestable
            for #struct_name #ty_generics #where_clause
        {
            const VALUE: bool = #testable_value;
        }
        impl #impl_generics
            ::nebula_credential::contract::plugin_capability_report::IsDynamic
            for #struct_name #ty_generics #where_clause
        {
            const VALUE: bool = #dynamic_value;
        }
    };

    // Per Tech Spec §15.8 + PR #582 review (CodeRabbit): emit a
    // compile-time parity assertion for every declared capability so
    // the macro cannot self-attest a capability without an actual
    // sub-trait impl. A user who writes `capabilities(refreshable)`
    // but forgets `impl Refreshable for X` previously passed
    // expansion (the IsRefreshable::VALUE = true seemed to work) and
    // failed only at engine dispatch — recreating the §15.8
    // self-attestation anti-pattern that the registry rewrite was
    // meant to close. With these assertions the missing sub-trait
    // impl surfaces as `the trait bound \`X: Refreshable\` is not
    // satisfied` at the macro-emit site, which is the right failure
    // surface for plugin authors.
    //
    // Each block is a never-called private fn so it consumes no
    // runtime cycles; the bound is a compile-time check only.
    let mut parity_checks = quote! {};
    if caps.interactive {
        parity_checks = quote! {
            #parity_checks
            const _: fn() = || {
                fn assert_capability<T: ::nebula_credential::Interactive>() {}
                assert_capability::<#struct_name #ty_generics>();
            };
        };
    }
    if caps.refreshable {
        parity_checks = quote! {
            #parity_checks
            const _: fn() = || {
                fn assert_capability<T: ::nebula_credential::Refreshable>() {}
                assert_capability::<#struct_name #ty_generics>();
            };
        };
    }
    if caps.revocable {
        parity_checks = quote! {
            #parity_checks
            const _: fn() = || {
                fn assert_capability<T: ::nebula_credential::Revocable>() {}
                assert_capability::<#struct_name #ty_generics>();
            };
        };
    }
    if caps.testable {
        parity_checks = quote! {
            #parity_checks
            const _: fn() = || {
                fn assert_capability<T: ::nebula_credential::Testable>() {}
                assert_capability::<#struct_name #ty_generics>();
            };
        };
    }
    if caps.dynamic {
        parity_checks = quote! {
            #parity_checks
            const _: fn() = || {
                fn assert_capability<T: ::nebula_credential::Dynamic>() {}
                assert_capability::<#struct_name #ty_generics>();
            };
        };
    }

    // Emit `Credential` impl per Phase 5: `type Properties` resolved per
    // `PropertiesSource` (direct vs StaticProtocol-bridged); `resolve` body
    // delegates to `StaticProtocol::build` in protocol-mode and is a typed
    // `todo!()` placeholder in properties-mode (the user supplies their own
    // `resolve` via a separate inherent impl block — Rust's coherence
    // forbids splitting a single trait impl, so properties-mode users
    // omit `#[derive(Credential)]` and write the full `impl Credential`
    // by hand. The properties-mode here exists so diagnostics line up at
    // the macro call site if a user forgets the manual impl).
    let credential_impl = match &attrs.properties_source {
        PropertiesSource::ViaProtocol(protocol) => quote! {
            impl #impl_generics ::nebula_credential::Credential
                for #struct_name #ty_generics #where_clause
            {
                type Properties = <#protocol as ::nebula_credential::StaticProtocol>::Properties;
                type Scheme = #scheme;
                type State = #scheme;

                const KEY: &'static str = #key;

                fn metadata() -> ::nebula_credential::CredentialMetadata
                where
                    Self: Sized,
                {
                    #metadata_body
                }

                fn project(state: &#scheme) -> #scheme
                where
                    Self: Sized,
                {
                    state.clone()
                }

                fn resolve(
                    values: &::nebula_schema::FieldValues,
                    _ctx: &::nebula_credential::CredentialContext,
                ) -> impl ::std::future::Future<
                    Output = ::std::result::Result<
                        ::nebula_credential::resolve::ResolveResult<#scheme, ()>,
                        ::nebula_credential::CredentialError,
                    >,
                > + ::std::marker::Send
                where
                    Self: Sized,
                {
                    async {
                        let scheme =
                            <#protocol as ::nebula_credential::StaticProtocol>::build(values)?;
                        ::std::result::Result::Ok(
                            ::nebula_credential::resolve::ResolveResult::Complete(scheme),
                        )
                    }
                }
            }
        },
        PropertiesSource::Direct(properties) => quote! {
            impl #impl_generics ::nebula_credential::Credential
                for #struct_name #ty_generics #where_clause
            {
                type Properties = #properties;
                type Scheme = #scheme;
                type State = #scheme;

                const KEY: &'static str = #key;

                fn metadata() -> ::nebula_credential::CredentialMetadata
                where
                    Self: Sized,
                {
                    #metadata_body
                }

                fn project(state: &#scheme) -> #scheme
                where
                    Self: Sized,
                {
                    state.clone()
                }

                fn resolve(
                    _values: &::nebula_schema::FieldValues,
                    _ctx: &::nebula_credential::CredentialContext,
                ) -> impl ::std::future::Future<
                    Output = ::std::result::Result<
                        ::nebula_credential::resolve::ResolveResult<#scheme, ()>,
                        ::nebula_credential::CredentialError,
                    >,
                > + ::std::marker::Send
                where
                    Self: Sized,
                {
                    async move {
                        ::std::todo!(
                            "implement `Credential::resolve` for `{}` — the macro `properties` \
                             mode does not synthesize a resolver. Either: \
                             (a) skip `#[derive(Credential)]` and write the full `impl Credential` \
                                 by hand (preferred for non-trivial logic), or \
                             (b) switch to `protocol = TypePath` so the macro can delegate to \
                                 `<TypePath as StaticProtocol>::build`.",
                            ::std::stringify!(#struct_name),
                        )
                    }
                }
            }
        },
    };

    Ok(quote! {
        #credential_impl

        #deps_impl

        #capability_impls

        #parity_checks
    })
}
