use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{DeriveInput, Type, parse_macro_input};

use crate::support::{attrs, diag, utils};

pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand(input) {
        Ok(ts) => ts,
        Err(e) => diag::to_compile_error(e),
    }
}

fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let fields = utils::require_named_fields(&input)?;
    let mut param_defs = Vec::new();

    for field in &fields.named {
        let field_name = field.ident.as_ref().expect("named field");
        let param_attrs = attrs::parse_attrs(&field.attrs, "param")?;

        if param_attrs.has_flag("skip") {
            continue;
        }

        let def = generate_param_def(field_name, &field.ty, &param_attrs)?;
        param_defs.push(def);
    }

    let param_count = param_defs.len();
    let expanded = quote! {
        impl #impl_generics #struct_name #ty_generics #where_clause {
            /// Returns the parameter collection describing all fields.
            pub fn parameters() -> ::nebula_parameter::collection::ParameterCollection {
                use ::nebula_parameter::collection::ParameterCollection;

                ParameterCollection::new()
                    #(.with(#param_defs))*
            }

            /// Returns the number of parameters.
            pub const fn param_count() -> usize {
                #param_count
            }
        }
    };

    Ok(expanded.into())
}

fn generate_param_def(
    field_name: &syn::Ident,
    field_type: &Type,
    attrs: &attrs::AttrArgs,
) -> syn::Result<TokenStream2> {
    let key = field_name.to_string();
    let name = attrs
        .get_string("name")
        .unwrap_or_else(|| field_name.to_string());
    let description = attrs.get_string("description");
    let required = attrs.has_flag("required");
    let secret = attrs.has_flag("secret");
    let default = attrs.get_value("default");

    let kind = infer_kind(field_type, secret);
    let (init_expr, default_setter) = match kind {
        ParamKind::Secret => (
            quote!(::nebula_parameter::types::SecretParameter::new(#key, #name)),
            default_secret_setter(default),
        ),
        ParamKind::Checkbox => (
            quote!(::nebula_parameter::types::CheckboxParameter::new(#key, #name)),
            default_checkbox_setter(default),
        ),
        ParamKind::Number => (
            quote!(::nebula_parameter::types::NumberParameter::new(#key, #name)),
            default_number_setter(default),
        ),
        ParamKind::Text => (
            quote!(::nebula_parameter::types::TextParameter::new(#key, #name)),
            default_text_setter(default),
        ),
    };

    let description_setter = description
        .map(|value| quote!(param.metadata.description = Some(#value.to_string());))
        .unwrap_or_default();

    let enum_wrap = match kind {
        ParamKind::Secret => quote!(::nebula_parameter::def::ParameterDef::Secret(param)),
        ParamKind::Checkbox => quote!(::nebula_parameter::def::ParameterDef::Checkbox(param)),
        ParamKind::Number => quote!(::nebula_parameter::def::ParameterDef::Number(param)),
        ParamKind::Text => quote!(::nebula_parameter::def::ParameterDef::Text(param)),
    };

    Ok(quote! {{
        let mut param = #init_expr;
        #description_setter
        param.metadata.required = #required;
        #default_setter
        #enum_wrap
    }})
}

fn default_text_setter(default: Option<&attrs::AttrValue>) -> TokenStream2 {
    match default {
        Some(attrs::AttrValue::Lit(syn::Lit::Str(value))) => {
            quote!(param.default = Some(#value.to_string());)
        }
        _ => quote! {},
    }
}

fn default_secret_setter(default: Option<&attrs::AttrValue>) -> TokenStream2 {
    default_text_setter(default)
}

fn default_number_setter(default: Option<&attrs::AttrValue>) -> TokenStream2 {
    match default {
        Some(attrs::AttrValue::Lit(syn::Lit::Int(value))) => {
            quote!(param.default = Some((#value) as f64);)
        }
        Some(attrs::AttrValue::Lit(syn::Lit::Float(value))) => {
            quote!(param.default = Some(#value);)
        }
        _ => quote! {},
    }
}

fn default_checkbox_setter(default: Option<&attrs::AttrValue>) -> TokenStream2 {
    match default {
        Some(attrs::AttrValue::Lit(syn::Lit::Bool(value))) => {
            quote!(param.default = Some(#value);)
        }
        _ => quote! {},
    }
}

#[derive(Clone, Copy, Debug)]
enum ParamKind {
    Text,
    Number,
    Checkbox,
    Secret,
}

fn infer_kind(ty: &Type, secret: bool) -> ParamKind {
    if secret {
        return ParamKind::Secret;
    }

    let base = unwrap_option(ty);
    let text = is_type(base, &["String", "str"]);
    let checkbox = is_type(base, &["bool"]);
    let number = is_type(
        base,
        &[
            "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
            "f32", "f64",
        ],
    );

    if checkbox {
        ParamKind::Checkbox
    } else if number {
        ParamKind::Number
    } else if text {
        ParamKind::Text
    } else {
        ParamKind::Text
    }
}

fn unwrap_option<'a>(ty: &'a Type) -> &'a Type {
    if let Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.last()
        && segment.ident == "Option"
        && let syn::PathArguments::AngleBracketed(args) = &segment.arguments
        && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
    {
        return inner;
    }

    ty
}

fn is_type(ty: &Type, names: &[&str]) -> bool {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| names.iter().any(|name| segment.ident == *name))
            .unwrap_or(false),
        Type::Reference(reference) => is_type(&reference.elem, names),
        _ => false,
    }
}
