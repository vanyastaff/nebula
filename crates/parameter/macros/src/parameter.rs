//! Parameters derive macro implementation.

use nebula_macro_support::{attrs, diag, utils};
use proc_macro::TokenStream;
use quote::quote;
use syn::parse_macro_input;

use crate::param_attrs::{ParameterAttrs, ValidateAttrs};

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
    let mut default_fields = Vec::new();
    let mut has_any_default = false;

    for field in &fields.named {
        let field_name = field.ident.as_ref().expect("named field");
        let field_type = &field.ty;
        let param_attrs = attrs::parse_attrs(&field.attrs, "param")?;

        if ParameterAttrs::is_skip(&param_attrs) {
            default_fields.push(quote! { #field_name: ::core::default::Default::default() });
            continue;
        }

        let attrs = ParameterAttrs::parse(&param_attrs, field_name)?;

        if let Some(default_val) = &attrs.default {
            has_any_default = true;
            let expr = attr_value_to_default_expr(default_val, field_type);
            default_fields.push(quote! { #field_name: #expr });
        } else {
            default_fields.push(quote! { #field_name: ::core::default::Default::default() });
        }

        // Parse #[validate(...)] if present
        let validate_args = attrs::parse_attrs(&field.attrs, "validate")?;
        let validate_attrs = ValidateAttrs::parse(&validate_args)?;

        let mut def = attrs.param_def_expr(field_type)?;

        if validate_attrs.required {
            def = quote! { #def.required() };
        }
        for rule_setter in validate_attrs.rule_exprs() {
            def = quote! { #def #rule_setter };
        }

        param_defs.push(def);
    }

    let default_impl = if has_any_default {
        quote! {
            impl #impl_generics ::core::default::Default for #struct_name #ty_generics #where_clause {
                fn default() -> Self {
                    Self { #(#default_fields,)* }
                }
            }
        }
    } else {
        quote! {}
    };

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

        #default_impl
    };

    Ok(expanded)
}

/// Convert an `AttrValue` default to a Rust expression suitable for a `Default` impl field.
fn attr_value_to_default_expr(val: &attrs::AttrValue, ty: &syn::Type) -> proc_macro2::TokenStream {
    let type_name = crate::param_attrs::type_to_string(ty);
    match val {
        attrs::AttrValue::Lit(syn::Lit::Str(s)) => {
            let v = s.value();
            match type_name.as_str() {
                "String" => quote! { #v.to_owned() },
                _ => quote! { #v.into() },
            }
        }
        attrs::AttrValue::Lit(syn::Lit::Int(i)) => {
            quote! { #i }
        }
        attrs::AttrValue::Lit(syn::Lit::Float(f)) => {
            quote! { #f }
        }
        attrs::AttrValue::Lit(syn::Lit::Bool(b)) => {
            let v = b.value;
            quote! { #v }
        }
        attrs::AttrValue::Ident(i) => {
            quote! { #i }
        }
        attrs::AttrValue::Tokens(tokens) => {
            quote! { #tokens }
        }
        _ => quote! { ::core::default::Default::default() },
    }
}
