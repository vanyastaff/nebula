//! `#[derive(Action)]` macro implementation — Variant A.
//!
//! Emits:
//!
//! - `impl Component for Foo` with the key, `Config` and static `metadata`, and `impl Action
//!   for Foo` with `Output`, `dependencies` and, for `read_only`, `EFFECT_CONTRACT` (no schema
//!   methods — the `Config`/`Output: HasSchema` bound is the single source of truth).
//! - `impl FromWorkflowNode for Foo` when the struct has at least one `#[resource]` /
//!   `#[credential]` field, or when the struct is a unit struct (no-op factory).
//!
//! Field-level attributes recognised:
//!
//! - `#[resource]` / `#[resource(key = "...")]` — declares a resource slot. Field type must be
//!   `ResourceHandle<R>` (optionally wrapped in `Option<...>`); a `ResourceGuard<R>` lease is
//!   refused since 0.27.0.
//! - `#[credential]` / `#[credential(key = "...")]` — declares a credential slot. Field type must
//!   be `CredentialGuard<C>` (optionally wrapped in `Option<...>`); `Lazy` wrappers are refused.

use nebula_macro_support::{attrs, diag};
use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

use nebula_macro_support::slot::{self as slots, SlotField};

use crate::action_attrs::ActionAttrs;

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
    let attrs = ActionAttrs::parse(&attr_args, struct_name, &input.attrs)?;

    if let Fields::Unnamed(_) = fields {
        return Err(syn::Error::new_spanned(
            fields,
            "#[derive(Action)] does not support tuple structs \
             — use a named-field struct or a unit struct",
        ));
    }
    let slots = slots::parse_slots(fields)?;
    let dependencies = slots::emit_dependencies(&quote!(::nebula_action), &slots);

    let metadata_init = attrs.component.metadata_expr(&quote!(::nebula_action));
    let (key_const, key_check) = attrs.key_items();

    let config_ty = &attrs.component.config;
    let output_ty = &attrs.output;
    let effect_contract = attrs.effect_contract_item();

    // Check that when there are non-slot fields, all required field types are
    // resolvable from the slot factory body. We don't fully enforce this here
    // (rustc will complain at impl time) — but the macro emits a structural
    // factory body that constructs the struct via field-name init shorthand,
    // so any non-slot field would need a `Default` impl. That's acceptable
    // — the user gets a clear rustc error pointing at their non-slot field.
    let needs_factory = matches!(fields, Fields::Named(_)) || matches!(fields, Fields::Unit);

    let action_impl = quote! {
        #key_check

        impl #impl_generics ::nebula_action::Component for #struct_name #ty_generics #where_clause {
            type Kind = ::nebula_action::kind::Action;
            type Config = #config_ty;

            #key_const

            fn metadata() -> ::nebula_action::Metadata {
                #metadata_init
            }

            fn dependencies() -> ::nebula_action::Dependencies {
                #dependencies
            }
        }

        impl #impl_generics ::nebula_action::Action for #struct_name #ty_generics #where_clause {
            type Output = #output_ty;
            #effect_contract
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
    slots: &[SlotField],
) -> syn::Result<proc_macro2::TokenStream> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let fields = match &input.data {
        Data::Struct(s) => &s.fields,
        _ => unreachable!("checked by expand()"),
    };

    let resolution_block = slots.iter().map(|slot| {
        let field = &slot.field_ident;
        let ty = &slot.ty;
        let key = &slot.key;
        quote! {
            let #field = <#ty as ::nebula_action::FromSlot>::from_slot(node, ctx, #key).await?;
        }
    });
    let slot_idents: Vec<&syn::Ident> = slots.iter().map(|slot| &slot.field_ident).collect();

    // Build the struct constructor expression covering both slot fields
    // (resolved) and non-slot fields (default-initialised). This keeps the
    // factory functional for hybrid structs that have both kinds of fields.
    let constructor = match fields {
        Fields::Named(named) => {
            let inits = named.named.iter().map(|f| {
                let f_ident = f.ident.as_ref().expect("named field has ident");
                if slot_idents.contains(&f_ident) {
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
                    #(#resolution_block)*
                    Ok(#constructor)
                }
            }
        }
    })
}
