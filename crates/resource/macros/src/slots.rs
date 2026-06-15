//! `#[derive(Resource)]` macro implementation — slot model.
//!
//! ## Two-derive pattern
//!
//! Resource authors use two derives:
//!
//! 1. `#[derive(Resource)]` (this macro) — emits the slot plumbing:
//!    - `impl DeclaresDependencies` enumerating `#[credential]` slot fields.
//!    - An inherent `pub fn <field>_slot(&self) -> Option<Arc<...>>` accessor per slot.
//!    - `impl HasCredentialSlots` with the order-sensitive positional fold
//!      and a `declares_credential_slots()` that is `true` iff the struct
//!      has at least one `#[credential]` field.
//!
//! 2. Hand-written `impl Provider` — the implementor supplies `key()`, the two
//!    associated types (`Config`, `Instance`), and the lifecycle methods
//!    (`create`, optionally `check`, `shutdown`, `destroy`, hooks).
//!
//! The macro **never** emits any `Provider` item, `todo!()`, `key()`, or
//! `metadata()`. There is no `#[resource(...)]` container attribute.
//!
//! ## Field attributes
//!
//! `#[credential]` / `#[credential(key = "...", purpose = "...")]`
//!
//! The field type must be **exactly** `SlotCell<CredentialGuard<C>>` or the
//! alias `CredentialSlot<C>` (path-tail matching). Any other type is a
//! compile error at the field span naming the two accepted shapes.
//!
//! The `key = "..."` literal is validated at expansion time: invalid keys
//! produce a compile error at the literal span.
//!
//! ## Slot-less structs
//!
//! Deriving on a struct with no `#[credential]` fields is legal. The macro
//! emits empty `DeclaresDependencies` and `HasCredentialSlots { fn
//! credential_slot_epoch → 0, fn declares_credential_slots → false }`
//! implementations. No accessors are emitted.
//!
//! ## Rejected forms
//!
//! - Enums and unions: compile error at the type identifier span.
//! - Tuple structs with a `#[credential]` field: compile error.
//! - Slot field with wrong type: compile error at the field type.

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, parse_macro_input};

use crate::{field_slots, topology_attr};

pub(crate) fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand(input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // Reject enums and unions at the type ident span.
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

    // Reject `#[resource(...)]` container attributes — deleted in this derive.
    for attr in &input.attrs {
        if attr.path().is_ident("resource") {
            return Err(syn::Error::new_spanned(
                attr,
                "#[resource(...)] container attribute is not accepted by \
                 #[derive(Resource)] — it was only used by an older retired derive \
                 which emitted a todo!() body. \
                 Write `impl Provider for ... { ... }` directly instead.",
            ));
        }
    }

    // Parse the optional `#[topology(Kind)]` container attribute.
    let topology_kind = topology_attr::parse_topology_attr(&input.attrs)?;

    let slots = field_slots::parse_credential_slot_fields_slots(fields)?;
    let slot_registrations = field_slots::emit_slot_field_registrations_with_purpose(&slots);
    let slot_accessors = field_slots::emit_slot_accessors(&slots);
    let credential_slot_epoch_body = field_slots::emit_credential_slot_epoch_body(&slots);
    // Type-level signal: `true` iff the struct declared at least one
    // `#[credential]` field. Emitted explicitly (rather than relying on the
    // trait default) so the slot-less case reads `false` at the impl site.
    let declares_credential_slots = !slots.is_empty();

    let has_credential_slots_impl = quote! {
        impl #impl_generics ::nebula_resource::HasCredentialSlots for #struct_name #ty_generics #where_clause {
            fn credential_slot_epoch(&self) -> u64 {
                #credential_slot_epoch_body
            }

            fn declares_credential_slots() -> bool {
                #declares_credential_slots
            }
        }
    };

    let deps_impl = quote! {
        impl #impl_generics ::nebula_core::DeclaresDependencies for #struct_name #ty_generics #where_clause {
            fn dependencies() -> ::nebula_core::Dependencies {
                ::nebula_core::Dependencies::new()
                    #slot_registrations
            }
        }
    };

    let slot_accessor_impl = if slot_accessors.is_empty() {
        quote! {}
    } else {
        quote! {
            impl #impl_generics #struct_name #ty_generics #where_clause {
                #slot_accessors
            }
        }
    };

    // Emit `<Name>Factory` only when `#[topology(Kind)]` is present.
    let factory_item = match topology_kind {
        Some(kind) => emit_factory(struct_name, kind),
        None => quote! {},
    };

    Ok(quote! {
        #has_credential_slots_impl
        #deps_impl
        #slot_accessor_impl
        #factory_item
    })
}

/// Emit the `<Name>Factory` newtype for the given topology kind.
///
/// The factory stores an `Arc<dyn ResourceFactory>` erased at construction
/// so the unnameable `KindActivator<R, FRes, FTopo>` closure types never
/// appear in the generated item. All `ResourceFactory` methods delegate to
/// the inner arc.
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

    // Build the topology factory closure based on the chosen kind.
    let topology_factory = match kind {
        topology_attr::TopologyKind::Resident => quote! {
            || ::nebula_resource::topology::Resident::<#struct_name>::new(
                ::nebula_resource::topology::resident::config::Config::default(),
            )
        },
        topology_attr::TopologyKind::Pooled => quote! {
            || ::nebula_resource::topology::Pooled::<#struct_name>::new(
                ::nebula_resource::topology::pooled::config::Config::default(),
                // fingerprint = 0: the activator registry is the canonical source
                // of per-kind fingerprints; the factory-emitted topology is the
                // default, not a config snapshot.
                0,
            )
        },
    };

    let kind_str = kind.as_str();

    quote! {
        /// Factory for [`#struct_name`] — implements
        /// [`nebula_resource::ResourceFactory`] with the
        #[doc = #kind_str]
        /// topology baked in.
        ///
        /// Emitted by `#[derive(Resource)]` when `#[topology(`
        #[doc = #kind_str]
        /// )]` is present.  Pass `Arc::new(<Name>Factory::new())` to
        /// `Plugin::resources()` or `ResourceActivatorRegistry::insert`.
        #[derive(Debug, Clone)]
        pub struct #factory_name {
            inner: ::std::sync::Arc<dyn ::nebula_resource::ResourceFactory>,
        }

        impl #factory_name {
            /// Build a new factory with the default topology configuration.
            #[must_use]
            pub fn new() -> Self {
                use ::nebula_resource::factory::KindActivator;
                let activator = KindActivator::<#struct_name, _, _>::new(
                    || #struct_name::default(),
                    #topology_factory,
                );
                Self {
                    inner: ::std::sync::Arc::new(activator),
                }
            }
        }

        impl ::std::default::Default for #factory_name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl ::nebula_resource::ResourceFactory for #factory_name {
            fn key(&self) -> ::nebula_core::ResourceKey {
                self.inner.key()
            }

            fn metadata(&self) -> ::nebula_resource::ResourceMetadata {
                self.inner.metadata()
            }

            fn validate(
                &self,
                config_json: ::serde_json::Value,
            ) -> ::std::result::Result<(), ::nebula_resource::Error> {
                self.inner.validate(config_json)
            }

            fn register<'__factory_lt>(
                &'__factory_lt self,
                manager: &'__factory_lt ::nebula_resource::Manager,
                request: ::nebula_resource::factory::RegisterRequest<'__factory_lt>,
            ) -> ::nebula_resource::factory::BoxFut<
                '__factory_lt,
                ::std::result::Result<::nebula_resource::SlotIdentity, ::nebula_resource::Error>,
            > {
                self.inner.register(manager, request)
            }
        }
    }
}
