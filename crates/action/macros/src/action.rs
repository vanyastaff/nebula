//! `#[derive(Action)]` macro implementation — Variant A (ADR-0043 §6).
//!
//! Emits:
//! - `impl Action for Foo` with static `metadata`, `input_schema`, `output_schema`, `dependencies`
//!   functions.
//! - `impl FromWorkflowNode for Foo` (when the struct has at least one `#[resource]` /
//!   `#[credential]` field, or when the struct is a unit struct — in that case a no-op factory).
//!
//! Field-level attributes recognised:
//! - `#[resource]` / `#[resource(key = "...")]` — declares a resource slot. Field type must be
//!   `ResourceGuard<R>` (optionally wrapped in `Option<...>` and/or `Lazy<...>`).
//! - `#[credential]` / `#[credential(key = "...")]` — declares a credential slot. Field type must
//!   be `CredentialGuard<C>` (optionally wrapped in `Option<...>` and/or `Lazy<...>`).

use nebula_macro_support::{attrs, diag, utils};
use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

use crate::{action_attrs::ActionAttrs, field_slots};

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

    let fields = match &input.data {
        Data::Struct(s) => &s.fields,
        Data::Enum(_) | Data::Union(_) => {
            return Err(syn::Error::new(
                struct_name.span(),
                "#[derive(Action)] can only be used on structs",
            ));
        },
    };

    let attr_args = attrs::parse_attrs(&input.attrs, "action")?;
    let description_fallback = utils::doc_string(&input.attrs);
    let description_fallback = if description_fallback.is_empty() {
        None
    } else {
        Some(description_fallback)
    };

    let attrs = ActionAttrs::parse(&attr_args, struct_name, description_fallback)?;

    // Parse field-level slot attributes.
    let slots = field_slots::parse_slot_fields(fields)?;
    let slot_registrations = field_slots::emit_slot_field_registrations(&slots);

    let metadata_init = attrs.metadata_init_expr();

    let input_ty = &attrs.input;
    let output_ty = &attrs.output;

    // Check that when there are non-slot fields, all required field types are
    // resolvable from the slot factory body. We don't fully enforce this here
    // (rustc will complain at impl time) — but the macro emits a structural
    // factory body that constructs the struct via field-name init shorthand,
    // so any non-slot field would need a `Default` impl. That's acceptable
    // — the user gets a clear rustc error pointing at their non-slot field.
    let needs_factory = matches!(fields, Fields::Named(_)) || matches!(fields, Fields::Unit);

    let action_impl = quote! {
        impl #impl_generics ::nebula_action::Action for #struct_name #ty_generics #where_clause {
            type Input = #input_ty;
            type Output = #output_ty;

            fn metadata() -> &'static ::nebula_action::ActionMetadata {
                static METADATA: ::std::sync::OnceLock<::nebula_action::ActionMetadata> =
                    ::std::sync::OnceLock::new();
                METADATA.get_or_init(|| #metadata_init)
            }

            fn input_schema() -> &'static ::nebula_action::ValidSchema {
                static SCHEMA: ::std::sync::OnceLock<::nebula_action::ValidSchema> =
                    ::std::sync::OnceLock::new();
                SCHEMA.get_or_init(|| {
                    <#input_ty as ::nebula_schema::HasSchema>::schema()
                })
            }

            fn output_schema() -> &'static ::nebula_action::ValidSchema {
                static SCHEMA: ::std::sync::OnceLock<::nebula_action::ValidSchema> =
                    ::std::sync::OnceLock::new();
                SCHEMA.get_or_init(|| {
                    <#output_ty as ::nebula_schema::HasSchema>::schema()
                })
            }

            fn dependencies() -> &'static ::nebula_core::Dependencies {
                static DEPS: ::std::sync::OnceLock<::nebula_core::Dependencies> =
                    ::std::sync::OnceLock::new();
                DEPS.get_or_init(|| {
                    ::nebula_core::Dependencies::new()
                        #slot_registrations
                })
            }
        }
    };

    let factory_impl = if needs_factory {
        emit_factory_impl(&input, &slots)?
    } else {
        proc_macro2::TokenStream::new()
    };

    Ok(quote! {
        #action_impl
        #factory_impl
    })
}

fn emit_factory_impl(
    input: &DeriveInput,
    slots: &[field_slots::ParsedSlotField],
) -> syn::Result<proc_macro2::TokenStream> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let fields = match &input.data {
        Data::Struct(s) => &s.fields,
        _ => unreachable!("checked by expand()"),
    };

    let (resolution_block, slot_idents) = field_slots::emit_slot_resolution_block(slots);

    // Build the struct constructor expression covering both slot fields
    // (resolved) and non-slot fields (default-initialised). This keeps the
    // factory functional for hybrid structs that have both kinds of fields.
    let constructor = match fields {
        Fields::Named(named) => {
            let inits = named.named.iter().map(|f| {
                let f_ident = f.ident.as_ref().expect("named field has ident");
                if slot_idents.iter().any(|s| s == f_ident) {
                    // Slot field — resolved via local of the same name.
                    quote! { #f_ident }
                } else {
                    // Non-slot field — must implement Default (rustc will catch this).
                    quote! { #f_ident: ::std::default::Default::default() }
                }
            });
            quote! { #struct_name { #(#inits),* } }
        },
        Fields::Unit => quote! { #struct_name },
        Fields::Unnamed(_) => {
            return Err(syn::Error::new_spanned(
                fields,
                "#[derive(Action)] does not support tuple structs",
            ));
        },
    };

    Ok(quote! {
        impl #impl_generics ::nebula_action::FromWorkflowNode
            for #struct_name #ty_generics #where_clause
        {
            type Error = ::nebula_action::ActionError;

            fn from_workflow_node<'__a>(
                node: &'__a ::nebula_workflow::NodeDefinition,
                ctx: &'__a (dyn ::nebula_action::ActionContext + '__a),
            ) -> impl ::std::future::Future<Output = ::std::result::Result<Self, Self::Error>>
                + ::std::marker::Send
                + '__a
            {
                async move {
                    let _ = node;
                    let _ = ctx;
                    #resolution_block
                    Ok(#constructor)
                }
            }
        }
    })
}
