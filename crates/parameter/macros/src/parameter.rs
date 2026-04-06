//! Parameters derive macro implementation.

use proc_macro::TokenStream;
use quote::quote;
use syn::parse_macro_input;

use nebula_macro_support::{attrs, diag, utils};

use crate::param_attrs::ParameterAttrs;

pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);

    match expand(input) {
        Ok(ts) => ts.into(),
        Err(e) => diag::to_compile_error(e).into(),
    }
}

fn expand(input: syn::DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let fields = utils::require_named_fields(&input)?;
    let mut param_defs = Vec::new();

    for field in &fields.named {
        let field_name = field.ident.as_ref().expect("named field");
        let field_type = &field.ty;
        let param_attrs = attrs::parse_attrs(&field.attrs, "param")?;

        if ParameterAttrs::is_skip(&param_attrs) {
            continue;
        }

        let attrs = ParameterAttrs::parse(&param_attrs, field_name)?;
        let def = attrs.param_def_expr(field_type)?;
        param_defs.push(def);
    }

    let param_count = param_defs.len();
    let expanded = quote! {
        impl #impl_generics ::nebula_parameter::HasParameters for #struct_name #ty_generics #where_clause {
            fn parameters() -> ::nebula_parameter::collection::ParameterCollection {
                ::nebula_parameter::collection::ParameterCollection::new()
                    #(.add(#param_defs))*
            }
        }

        impl #impl_generics ::nebula_parameter::InferParameterType for #struct_name #ty_generics #where_clause {
            fn into_parameter(id: &str) -> ::nebula_parameter::parameter::Parameter {
                let nested = <Self as ::nebula_parameter::HasParameters>::parameters().into_vec();
                ::nebula_parameter::parameter::Parameter::object_with(id, nested)
            }
        }

        impl #impl_generics #struct_name #ty_generics #where_clause {
            /// Returns the parameter collection describing all fields.
            pub fn parameters() -> ::nebula_parameter::collection::ParameterCollection {
                <Self as ::nebula_parameter::HasParameters>::parameters()
            }

            /// Returns the number of parameters.
            pub const fn param_count() -> usize {
                #param_count
            }
        }
    };

    Ok(expanded)
}
