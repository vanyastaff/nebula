//! `#[derive(Resource)]` macro implementation.
//!
//! The derive writes everything a resource declares about itself; the author
//! writes only `impl Provider` (its instance, topology and lifecycle):
//!
//! - `impl Component` from `#[resource(key, name, description, version,
//!   config)]`, with `dependencies()` listing the `#[slot]` fields.
//! - `impl HasCredentialSlots`: the order-sensitive positional epoch fold
//!   and the install, projection, fence and revoke arms per slot.
//! - An inherent `pub fn <field>_slot(&self)` accessor and a `SLOT_<KEY>`
//!   constant per slot.
//! - `impl PinSlots` for the managed call facade: `Pinned = ()` for a
//!   slot-less struct, otherwise a generated `<Name>PinnedSlots` holding
//!   each slot's guard `Arc` loaded once per unit, read through a
//!   `<field>()` accessor (generic structs with slots get none).
//! - `<Name>Factory` when `#[topology(Kind)]` is present.
//!
//! A `#[slot]` field's type must implement `nebula_resource::ResourceSlot`
//! (today `CredentialSlot<C>`); the derive never parses it.

use nebula_macro_support::{
    attrs,
    component::{COMPONENT_KEYS, ComponentAttrs},
    slot::{self as slot_fields, SlotField},
};
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, parse_macro_input};

use crate::topology_attr;

pub(crate) fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand(input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let struct_name = &input.ident;

    let fields = match &input.data {
        Data::Struct(s) => &s.fields,
        Data::Enum(_) => {
            return Err(syn::Error::new_spanned(
                struct_name,
                "#[derive(Resource)] can only be used on structs, not enums",
            ));
        },
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                struct_name,
                "#[derive(Resource)] can only be used on structs, not unions",
            ));
        },
    };

    let attr_args = attrs::parse_attrs(&input.attrs, "resource")?;
    for item in &attr_args.items {
        let key = match item {
            attrs::AttrItem::KeyValue { key, .. }
            | attrs::AttrItem::Flag(key)
            | attrs::AttrItem::List { key, .. } => key,
        };
        if !COMPONENT_KEYS.iter().any(|allowed| key == allowed) {
            return Err(syn::Error::new_spanned(
                key,
                format!(
                    "unknown attribute `{key}` in #[resource(...)] — allowed keys: {}",
                    COMPONENT_KEYS.join(", "),
                ),
            ));
        }
    }
    let component = ComponentAttrs::parse(&attr_args, struct_name, "resource", &input.attrs)?;

    let topology_kind = topology_attr::parse_topology_attr(&input.attrs)?;

    let slots = slot_fields::parse_slots(fields)?;
    let component_impl = emit_component(&input, &component, &slots);
    let has_credential_slots_impl = emit_has_credential_slots(&input, &slots);
    let slot_accessor_impl = emit_slot_accessors(&input, &slots);
    let pin_slots_item = emit_pin_slots(&input, &slots);
    let factory_item = match topology_kind {
        Some(kind) => emit_factory(struct_name, kind),
        None => quote! {},
    };

    Ok(quote! {
        #component_impl
        #has_credential_slots_impl
        #slot_accessor_impl
        #pin_slots_item
        #factory_item
    })
}

/// Emit `impl Component`, with a definition-site check of the key literal.
fn emit_component(
    input: &DeriveInput,
    component: &ComponentAttrs,
    slots: &[SlotField],
) -> proc_macro2::TokenStream {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let krate = quote!(::nebula_resource);
    let key = &component.key;
    let config = &component.config;
    let metadata = component.metadata_expr(&krate);
    let dependencies = slot_fields::emit_dependencies(&krate, slots);
    quote! {
        const _: () = ::core::assert!(
            ::nebula_core::ResourceKey::is_valid_key_const(#key),
            "invalid resource key literal"
        );

        impl #impl_generics ::nebula_resource::Component for #struct_name #ty_generics #where_clause {
            type Kind = ::nebula_resource::kind::Resource;
            type Config = #config;

            const KEY: &'static str = #key;

            fn metadata() -> ::nebula_resource::Metadata {
                #metadata
            }

            fn dependencies() -> ::nebula_resource::Dependencies {
                #dependencies
            }
        }
    }
}

/// `<FieldType as ResourceSlot>::cell(&self.field)`.
fn cell(slot: &SlotField) -> proc_macro2::TokenStream {
    let field = &slot.field_ident;
    let ty = &slot.ty;
    quote! { <#ty as ::nebula_resource::ResourceSlot>::cell(&self.#field) }
}

/// Emit `impl HasCredentialSlots`: the epoch fold and the per-slot arms.
fn emit_has_credential_slots(input: &DeriveInput, slots: &[SlotField]) -> proc_macro2::TokenStream {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let epoch_body = if slots.is_empty() {
        quote! { 0 }
    } else {
        let generations = slots.iter().map(|slot| {
            let cell = cell(slot);
            quote! { #cell.generation() }
        });
        quote! {
            {
                const __NEBULA_SLOT_EPOCH_K: u64 = 0x0000_0100_0000_01b3;
                [ #(#generations),* ]
                    .into_iter()
                    .fold(0u64, |acc, slot_gen| {
                        acc.wrapping_mul(__NEBULA_SLOT_EPOCH_K).wrapping_add(slot_gen)
                    })
            }
        }
    };
    let declares_credential_slots = !slots.is_empty();
    let keys: Vec<&String> = slots.iter().map(|slot| &slot.key).collect();

    let install_arms = |conditional: bool| {
        let arms = slots.iter().map(move |slot| {
            let key = &slot.key;
            let ty = &slot.ty;
            let cell = cell(slot);
            let install = if conditional {
                quote! { #cell.install_projected_at_generation(expected_generation, metadata, ::std::sync::Arc::new(guard)) }
            } else {
                quote! { #cell.install_projected(metadata, ::std::sync::Arc::new(guard)) }
            };
            quote! {
                #key => {
                    let metadata = guard.metadata().clone();
                    let guard = <#ty as ::nebula_resource::ResourceSlot>::typed_guard(guard)?;
                    #install
                }
            }
        });
        quote! {
            match slot {
                #(#arms,)*
                _ => ::core::result::Result::Err(::nebula_resource::SlotInstallError::UnknownSlot),
            }
        }
    };
    let install_body = install_arms(false);
    let conditional_install_body = install_arms(true);
    let projection_arms = slots.iter().map(|slot| {
        let key = &slot.key;
        let cell = cell(slot);
        quote! { #key => ::core::option::Option::Some(#cell.projection_snapshot()) }
    });
    let fence_arms = slots.iter().map(|slot| {
        let key = &slot.key;
        let cell = cell(slot);
        quote! { #key => #cell.fence_projection_at_generation(expected_generation, fence) }
    });
    let metadata_arms = slots.iter().map(|slot| {
        let key = &slot.key;
        let cell = cell(slot);
        quote! { #key => #cell.projection_metadata() }
    });
    let revoke_arms = slots.iter().map(|slot| {
        let key = &slot.key;
        let cell = cell(slot);
        quote! { #key => ::core::result::Result::Ok(#cell.revoke()) }
    });

    quote! {
        impl #impl_generics ::nebula_resource::HasCredentialSlots for #struct_name #ty_generics #where_clause {
            fn credential_slot_epoch(&self) -> u64 {
                #epoch_body
            }

            fn declares_credential_slots() -> bool {
                #declares_credential_slots
            }

            fn credential_slot_names() -> &'static [&'static str] {
                &[#(#keys),*]
            }

            fn supports_credential_slot_projection(&self, slot: &str) -> bool {
                Self::credential_slot_names().contains(&slot)
            }

            fn install_credential_slot(
                &self,
                slot: &str,
                guard: ::nebula_credential::ErasedCredentialGuard,
            ) -> ::core::result::Result<
                ::nebula_resource::SlotUpdate,
                ::nebula_resource::SlotInstallError,
            > {
                #install_body
            }

            fn credential_slot_projection(
                &self,
                slot: &str,
            ) -> ::core::option::Option<(u64, ::core::option::Option<::nebula_credential::CredentialGuardMetadata>)> {
                match slot {
                    #(#projection_arms,)*
                    _ => ::core::option::Option::None,
                }
            }

            fn install_credential_slot_at_generation(
                &self,
                slot: &str,
                guard: ::nebula_credential::ErasedCredentialGuard,
                expected_generation: u64,
            ) -> ::core::result::Result<::nebula_resource::SlotUpdate, ::nebula_resource::SlotInstallError> {
                #conditional_install_body
            }

            fn fence_credential_slot_at_generation(
                &self,
                slot: &str,
                expected_generation: u64,
                fence: &mut dyn FnMut(),
            ) -> ::core::result::Result<(), ::nebula_resource::SlotInstallError> {
                match slot {
                    #(#fence_arms,)*
                    _ => ::core::result::Result::Err(::nebula_resource::SlotInstallError::UnknownSlot),
                }
            }

            fn credential_slot_metadata(
                &self,
                slot: &str,
            ) -> ::core::option::Option<::nebula_credential::CredentialGuardMetadata> {
                match slot {
                    #(#metadata_arms,)*
                    _ => ::core::option::Option::None,
                }
            }

            fn revoke_credential_slot(
                &self,
                slot: &str,
            ) -> ::core::result::Result<
                ::nebula_resource::SlotUpdate,
                ::nebula_resource::SlotInstallError,
            > {
                match slot {
                    #(#revoke_arms,)*
                    _ => ::core::result::Result::Err(::nebula_resource::SlotInstallError::UnknownSlot),
                }
            }
        }
    }
}

/// `SLOT_<KEY>`: the key upper-cased, non-alphanumerics as `_`.
fn slot_const_suffix(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Emit the `SLOT_<KEY>` constants and `<field>_slot()` accessors.
fn emit_slot_accessors(input: &DeriveInput, slots: &[SlotField]) -> proc_macro2::TokenStream {
    if slots.is_empty() {
        return quote! {};
    }
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let accessors = slots.iter().map(|slot| {
        let ty = &slot.ty;
        let key = &slot.key;
        let cell = cell(slot);
        let accessor = format_ident!("{}_slot", slot.field_ident);
        let const_ident = format_ident!("SLOT_{}", slot_const_suffix(key));
        let const_doc = format!(
            "The `{key}` slot key. Match `slot_name` against `Self::{const_ident}` in \
             `on_credential_refresh`/`on_credential_revoke` so a renamed slot field \
             becomes a compile error, not a silent no-op."
        );
        let accessor_doc =
            format!("The current value of the `{key}` slot, or `None` while it is unbound.");
        quote! {
            #[doc = #const_doc]
            pub const #const_ident: &str = #key;

            #[doc = #accessor_doc]
            #[must_use]
            pub fn #accessor(&self) -> ::core::option::Option<
                ::std::sync::Arc<<#ty as ::nebula_resource::ResourceSlot>::Guard>
            > {
                #cell.load()
            }
        }
    });
    quote! {
        impl #impl_generics #struct_name #ty_generics #where_clause {
            #(#accessors)*
        }
    }
}

/// Emit `impl PinSlots` — the per-unit slot snapshot of the managed call
/// facade.
///
/// A slot-less struct pins `()`. A struct with slots pins a generated
/// `<Name>PinnedSlots`: one private `Option<Arc<_>>` per slot, loaded once per
/// unit, read through a `<field>()` accessor. A generic struct with slots gets
/// no impl (the pinned type would have to be generic too); it implements
/// `PinSlots` by hand if it uses the facade.
fn emit_pin_slots(input: &DeriveInput, slots: &[SlotField]) -> proc_macro2::TokenStream {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    if slots.is_empty() {
        return quote! {
            impl #impl_generics ::nebula_resource::PinSlots for #struct_name #ty_generics #where_clause {
                type Pinned = ();

                fn pin_slots(&self) -> Self::Pinned {}
            }
        };
    }
    if !input.generics.params.is_empty() {
        return quote! {};
    }

    let vis = &input.vis;
    let pinned_name = format_ident!("{}PinnedSlots", struct_name, span = Span::call_site());
    let guard_types: Vec<proc_macro2::TokenStream> = slots
        .iter()
        .map(|slot| {
            let ty = &slot.ty;
            quote! { <#ty as ::nebula_resource::ResourceSlot>::Guard }
        })
        .collect();
    let cells: Vec<proc_macro2::TokenStream> = slots.iter().map(cell).collect();
    let fields: Vec<&syn::Ident> = slots.iter().map(|slot| &slot.field_ident).collect();
    let struct_doc = format!(
        "The slots of [`{struct_name}`] pinned for one managed call unit.\n\n\
         Loaded once, at the unit's first grant; every attempt of the unit reads the \
         same guards, and a rotation reaches the next unit. `None` for a slot that was \
         unbound at that grant. Emitted by `#[derive(Resource)]`."
    );
    let accessor_docs: Vec<String> = slots
        .iter()
        .map(|slot| {
            format!(
                "The `{}` slot pinned for this unit, or `None` if it was unbound.",
                slot.key
            )
        })
        .collect();
    let keys: Vec<&String> = slots.iter().map(|slot| &slot.key).collect();

    quote! {
        #[doc = #struct_doc]
        #vis struct #pinned_name {
            #( #fields: ::std::option::Option<::std::sync::Arc<#guard_types>>, )*
        }

        impl #pinned_name {
            #(
                #[doc = #accessor_docs]
                #[must_use]
                pub fn #fields(&self) -> ::std::option::Option<&#guard_types> {
                    self.#fields.as_deref()
                }
            )*
        }

        impl ::std::fmt::Debug for #pinned_name {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                formatter
                    .debug_struct(::std::stringify!(#pinned_name))
                    #( .field(#keys, &self.#fields.is_some()) )*
                    .finish()
            }
        }

        impl ::nebula_resource::PinSlots for #struct_name {
            type Pinned = #pinned_name;

            fn pin_slots(&self) -> Self::Pinned {
                #pinned_name {
                    #( #fields: #cells.load(), )*
                }
            }
        }
    }
}

/// Emit the `<Name>Factory` newtype for the given topology kind.
///
/// The factory stores an `Arc<dyn ResourceFactory>` erased at construction
/// so the unnameable `KindActivator<R, FRes, FTopo>` closure types never
/// appear in the generated item. The public trait is sealed; callers consume
/// the crate-issued capability through `into_contribution` rather than receiving
/// a downstream trait implementation.
///
/// The `new()` constructor is zero-argument: the topology is fixed by the
/// `#[topology(Kind)]` attribute at macro expansion time. No topology config
/// is threaded through because the derive-emitted factory is a registry
/// default — callers that need custom pool sizes build their own
/// `KindActivator` directly.
fn emit_factory(
    struct_name: &syn::Ident,
    kind: topology_attr::TopologyKind,
) -> proc_macro2::TokenStream {
    let factory_name = format_ident!("{}Factory", struct_name, span = Span::call_site());

    let kind_str = kind.as_str();

    quote! {
        /// Factory for [`#struct_name`] — implements
        /// [`nebula_resource::ResourceFactory`] with the
        #[doc = #kind_str]
        /// topology baked in.
        ///
        /// Emitted by `#[derive(Resource)]` when `#[topology(`
        #[doc = #kind_str]
        /// )]` is present. Call `into_contribution()` and pass the returned
        /// crate-issued capability to `Plugin::resources()` or
        /// `ResourceActivatorRegistry::insert`.
        #[derive(Clone)]
        pub struct #factory_name {
            inner: ::std::sync::Arc<dyn ::nebula_resource::ResourceFactory>,
        }

        impl ::std::fmt::Debug for #factory_name {
            fn fmt(
                &self,
                formatter: &mut ::std::fmt::Formatter<'_>,
            ) -> ::std::fmt::Result {
                formatter
                    .debug_struct(::std::stringify!(#factory_name))
                    .finish_non_exhaustive()
            }
        }

        impl #factory_name {
            /// Build a new factory; topology settings come from each registration.
            #[must_use]
            pub fn new() -> Self {
                use ::nebula_resource::factory::KindActivator;
                // Topology settings arrive per registration; the activator
                // publishes their schema. `kind` only selects `R::Topology`.
                let activator =
                    KindActivator::<#struct_name, _, _>::configurable(|| #struct_name::default());
                Self {
                    inner: ::std::sync::Arc::new(activator),
                }
            }

            /// Consume this wrapper and return the sealed erased factory.
            #[must_use]
            pub fn into_contribution(
                self,
            ) -> ::std::sync::Arc<dyn ::nebula_resource::ResourceFactory> {
                self.inner
            }
        }

        impl ::std::default::Default for #factory_name {
            fn default() -> Self {
                Self::new()
            }
        }

    }
}
