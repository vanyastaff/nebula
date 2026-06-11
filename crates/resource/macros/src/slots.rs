//! `#[derive(Resource)]` macro implementation — slot model.
//!
//! ## Two-derive pattern
//!
//! Resource authors use two derives:
//!
//! 1. `#[derive(Resource)]` (this macro) — emits the slot plumbing:
//!    - `impl DeclaresDependencies` enumerating `#[credential]` slot fields.
//!    - An inherent `pub fn <field>_slot(&self) -> Option<Arc<...>>` accessor per slot.
//!    - `impl HasCredentialSlots` with the order-sensitive positional fold.
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
//! credential_slot_epoch → 0 }` implementations. No accessors are emitted.
//!
//! ## Rejected forms
//!
//! - Enums and unions: compile error at the type identifier span.
//! - Tuple structs with a `#[credential]` field: compile error.
//! - Slot field with wrong type: compile error at the field type.

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, parse_macro_input};

use crate::field_slots;

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

    let slots = field_slots::parse_credential_slot_fields_slots(fields)?;
    let slot_registrations = field_slots::emit_slot_field_registrations_with_purpose(&slots);
    let slot_accessors = field_slots::emit_slot_accessors(&slots);
    let credential_slot_epoch_body = field_slots::emit_credential_slot_epoch_body(&slots);

    let has_credential_slots_impl = quote! {
        impl #impl_generics ::nebula_resource::HasCredentialSlots for #struct_name #ty_generics #where_clause {
            fn credential_slot_epoch(&self) -> u64 {
                #credential_slot_epoch_body
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

    Ok(quote! {
        #has_credential_slots_impl
        #deps_impl
        #slot_accessor_impl
    })
}
