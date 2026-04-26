//! Credential derive macro implementation (v2).

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

/// Parsed `#[credential(...)]` attributes for v2 derive.
struct CredentialAttrsV2 {
    key: String,
    name: String,
    scheme: syn::Type,
    protocol: syn::Type,
    icon: Option<String>,
    doc_url: Option<String>,
    capabilities: DeclaredCapabilities,
}

fn parse_credential_attrs(
    attr_args: &attrs::AttrArgs,
    struct_name: &syn::Ident,
) -> syn::Result<CredentialAttrsV2> {
    let key = attr_args.require_string("key", struct_name)?;
    let name = attr_args.require_string("name", struct_name)?;

    let scheme = attr_args.get_type("scheme")?.ok_or_else(|| {
        diag::error_spanned(
            struct_name,
            "#[derive(Credential)] requires `scheme = Type` attribute",
        )
    })?;

    let protocol = attr_args.get_type("protocol")?.ok_or_else(|| {
        diag::error_spanned(
            struct_name,
            "#[derive(Credential)] requires `protocol = Type` attribute",
        )
    })?;

    let icon = attr_args.get_string("icon");
    let doc_url = attr_args.get_string("doc_url");

    let capabilities = parse_capabilities(attr_args)?;

    Ok(CredentialAttrsV2 {
        key,
        name,
        scheme,
        protocol,
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
fn parse_capabilities(attr_args: &attrs::AttrArgs) -> syn::Result<DeclaredCapabilities> {
    let mut declared = DeclaredCapabilities::default();

    let Some(values) = attr_args.items.iter().find_map(|item| match item {
        AttrItem::List { key, values } if key == "capabilities" => Some(values),
        _ => None,
    }) else {
        return Ok(declared);
    };

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
        match ident.to_string().as_str() {
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
    let protocol = &attrs.protocol;
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
                    .schema(Self::schema())
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

    let expanded = quote! {
        impl #impl_generics ::nebula_credential::Credential
            for #struct_name #ty_generics #where_clause
        {
            type Input = <#protocol as ::nebula_credential::StaticProtocol>::Input;
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

        #deps_impl

        #capability_impls
    };

    Ok(expanded)
}
