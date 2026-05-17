//! `#[derive(Resource)]` macro implementation — Phase 4 / ADR-0044.
//!
//! Emits:
//! - `impl Resource for Foo` with `key()` returning the `#[resource(key = ...)]` value, and the
//!   four associated types (`Config`, `Runtime`, `Lease`, `Error`) read from the `#[resource(...)]`
//!   attribute. The `create` body is left as `todo!()` so the implementor must provide one — the
//!   macro emits the trait shape and the `key()` / metadata wiring only.
//! - `impl DeclaresDependencies for Foo` enumerating credential slot fields.
//! - An inherent `impl Foo` exposing one read accessor per credential slot:
//!   `fn <field>_slot(&self) -> Option<Arc<CredentialGuard<C>>>`. The macro
//!   adds no fields — the `SlotCell` is author-declared and the framework
//!   populates/rotates it through `&self`.
//!
//! Field-level attributes recognised:
//! - `#[credential]` / `#[credential(key = "...", purpose = "...")]` — declares a credential slot.
//!   Field type must be **exactly** `SlotCell<CredentialGuard<C>>`. The generated accessor emits a
//!   single fixed body that only fits that shape, so `Option<…>`- and `Lazy<…>`-wrapped slots are
//!   rejected at derive time (a compile error pointing at the field).

use nebula_macro_support::{attrs, diag};
use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, parse_macro_input};

use crate::{field_slots, resource_attrs::ResourceAttrs};

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
                "#[derive(Resource)] can only be used on structs",
            ));
        },
    };

    let attr_args = attrs::parse_attrs(&input.attrs, "resource")?;
    let attrs = ResourceAttrs::parse(&attr_args, struct_name)?;

    let slots = field_slots::parse_credential_slot_fields(fields)?;
    let slot_registrations = field_slots::emit_slot_field_registrations(&slots);
    let slot_accessors = field_slots::emit_slot_accessors(&slots);

    let key_lit = &attrs.key;
    let config_ty = &attrs.config;
    let runtime_ty = &attrs.runtime;
    let lease_ty = &attrs.lease;
    let error_ty = &attrs.error;
    let topology_ident = attrs.topology_ident();

    // Resource trait impl — the `create()` body must be supplied by the implementor.
    // The macro provides the trait shape + key() + metadata().
    let resource_impl = quote! {
        impl #impl_generics ::nebula_resource::Resource for #struct_name #ty_generics #where_clause {
            type Config = #config_ty;
            type Runtime = #runtime_ty;
            type Lease = #lease_ty;
            type Error = #error_ty;

            fn key() -> ::nebula_core::ResourceKey {
                ::nebula_core::ResourceKey::new(#key_lit)
                    .expect("invalid resource key in #[resource] attribute")
            }

            fn create(
                &self,
                _config: &Self::Config,
                _ctx: &::nebula_resource::ResourceContext,
            ) -> impl ::std::future::Future<Output = ::std::result::Result<Self::Runtime, Self::Error>> + Send {
                async move {
                    ::std::todo!(
                        "implement `Resource::create` for `{}`",
                        ::std::stringify!(#struct_name),
                    )
                }
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

    // Topology marker — the `topology = "..."` attribute is informational (used
    // by catalog / UI); the actual topology implementation comes from
    // separately impl'ing `Pooled` / `Resident` / `Service` / `Transport` /
    // `Exclusive` for the type. Emit a const that exposes the chosen topology
    // tag for runtime introspection.
    let topology_const = quote! {
        impl #impl_generics #struct_name #ty_generics #where_clause {
            /// The topology this resource was declared with (per
            /// `#[resource(topology = ...)]`). Used by catalog / UI for
            /// dependency-graph rendering.
            pub const RESOURCE_TOPOLOGY: ::nebula_resource::TopologyTag =
                ::nebula_resource::TopologyTag::#topology_ident;
        }
    };

    // Inherent read accessors over the author-declared `SlotCell` slot
    // fields. The macro adds no fields — the cell is declared by the author
    // and populated/rotated by the framework through `&self` (ADR-0044).
    let slot_accessor_impl = quote! {
        impl #impl_generics #struct_name #ty_generics #where_clause {
            #slot_accessors
        }
    };

    Ok(quote! {
        #resource_impl
        #deps_impl
        #topology_const
        #slot_accessor_impl
    })
}
