//! Parameters derive macro.
//!
//! Generates `parameters()` and `param_count()` from struct fields.

use proc_macro::TokenStream;
use quote::quote;
use syn::parse_macro_input;

use crate::support::{attrs, diag, utils};
use crate::types::ParameterAttrs;

pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);

    match expand(input) {
        Ok(ts) => ts,
        Err(e) => diag::to_compile_error(e),
    }
}

fn expand(input: syn::DeriveInput) -> syn::Result<TokenStream> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let fields = utils::require_named_fields(&input)?;
    let mut param_defs = Vec::new();

    for field in &fields.named {
        let field_name = field.ident.as_ref().expect("named field");
        let param_attrs = attrs::parse_attrs(&field.attrs, "param")?;

        if ParameterAttrs::is_skip(&param_attrs) {
            continue;
        }

        let attrs = ParameterAttrs::parse(&param_attrs, field_name)?;
        let def = attrs.param_def_expr()?;
        param_defs.push(def);
    }

    let param_count = param_defs.len();
    let expanded = quote! {
        impl #impl_generics #struct_name #ty_generics #where_clause {
            /// Returns the parameter schema describing all fields.
            pub fn parameters() -> ::nebula_parameter::schema::Schema {
                use ::nebula_parameter::schema::{Field, Schema};

                Schema::new()
                    #(.field(#param_defs))*
            }

            /// Returns the number of parameters.
            pub const fn param_count() -> usize {
                #param_count
            }
        }
    };

    Ok(expanded.into())
}
