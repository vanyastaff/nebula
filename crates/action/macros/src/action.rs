//! Action derive macro implementation.

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, parse_macro_input};

use nebula_macro_support::{attrs, diag, utils};

use crate::action_attrs::ActionAttrs;

pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand(input) {
        Ok(ts) => ts.into(),
        Err(e) => diag::to_compile_error(e).into(),
    }
}

fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    validate_struct(&input)?;

    let attr_args = attrs::parse_attrs(&input.attrs, "action")?;
    let description_fallback = utils::doc_string(&input.attrs);
    let description_fallback = if description_fallback.is_empty() {
        None
    } else {
        Some(description_fallback)
    };

    let attrs = ActionAttrs::parse(&attr_args, struct_name, description_fallback)?;

    let metadata_init = attrs.metadata_init_expr();
    let dependencies_impl =
        attrs.dependencies_impl_expr(struct_name, &impl_generics, &ty_generics, where_clause);

    let expanded = quote! {
        #dependencies_impl

        impl #impl_generics ::nebula_action::Action for #struct_name #ty_generics #where_clause {
            fn metadata(&self) -> &::nebula_action::metadata::ActionMetadata {
                use ::std::sync::OnceLock;

                static METADATA: OnceLock<::nebula_action::metadata::ActionMetadata> = OnceLock::new();
                METADATA.get_or_init(|| #metadata_init)
            }
        }
    };

    Ok(expanded)
}

fn validate_struct(input: &DeriveInput) -> syn::Result<()> {
    if !matches!(&input.data, Data::Struct(_)) {
        return Err(syn::Error::new(
            input.ident.span(),
            "Action derive can only be used on structs",
        ));
    }
    Ok(())
}
