//! EnumSelect derive macro implementation.
//!
//! Generates `HasSelectOptions` for enums, turning variants into `SelectOption`s.

use nebula_macro_support::{attrs, diag};
use proc_macro::TokenStream;
use quote::quote;
use syn::parse_macro_input;

pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);

    match expand(input) {
        Ok(ts) => ts.into(),
        Err(e) => diag::to_compile_error(e).into(),
    }
}

fn expand(input: syn::DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let enum_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let data = match &input.data {
        syn::Data::Enum(data) => data,
        _ => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "EnumSelect can only be derived on enums",
            ));
        }
    };

    let mut option_exprs = Vec::new();

    for variant in &data.variants {
        let variant_name = &variant.ident;

        // Only support unit variants (no fields)
        if !variant.fields.is_empty() {
            return Err(syn::Error::new_spanned(
                variant,
                "EnumSelect only supports unit variants (no fields)",
            ));
        }

        // Parse #[param(label = "...", description = "...")]
        let param_attrs = attrs::parse_attrs(&variant.attrs, "param")?;
        let label = param_attrs
            .get_string("label")
            .unwrap_or_else(|| variant_name.to_string());
        let description = param_attrs.get_string("description");

        let value_str = variant_name.to_string();

        let desc_setter = description
            .map(|d| quote!(.description(#d)))
            .unwrap_or_default();

        option_exprs.push(quote! {
            ::nebula_parameter::option::SelectOption::new(
                ::serde_json::Value::String(#value_str.to_string()),
                #label,
            )
            #desc_setter
        });
    }

    let expanded = quote! {
        impl #impl_generics ::nebula_parameter::HasSelectOptions for #enum_name #ty_generics #where_clause {
            fn select_options() -> ::std::vec::Vec<::nebula_parameter::option::SelectOption> {
                ::std::vec![
                    #(#option_exprs),*
                ]
            }
        }

        impl #impl_generics ::nebula_parameter::InferParameterType for #enum_name #ty_generics #where_clause {
            fn into_parameter(id: &str) -> ::nebula_parameter::parameter::Parameter {
                let mut param = ::nebula_parameter::parameter::Parameter::select(id);
                if let ::nebula_parameter::parameter_type::ParameterType::Select { options, .. } = &mut param.param_type {
                    *options = <Self as ::nebula_parameter::HasSelectOptions>::select_options();
                }
                param
            }
        }
    };

    Ok(expanded)
}
