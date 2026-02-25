//! Parsed `#[param(...)]` field attributes.
//!
//! [`ParameterAttrs`] holds per-field parameter metadata for the Parameters derive.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Ident, Lit, Result, Type};

use crate::support::attrs;

/// Parsed parameter field attributes.
///
/// Maps to `#[param(...)]` on struct fields.
#[derive(Debug, Clone)]
#[allow(dead_code)] // validation, options — for future use in param_def_expr
pub struct ParameterAttrs {
    /// Field key (from field name).
    pub key: String,
    /// Display name (defaults to key).
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Whether the parameter is required.
    pub required: bool,
    /// Whether the parameter is secret (sensitive).
    pub secret: bool,
    /// Default value if any.
    pub default: Option<attrs::AttrValue>,
    /// Validation rule (e.g. `"url"`, `"email"`).
    pub validation: Option<String>,
    /// Select options for dropdown.
    pub options: Option<Vec<String>>,
}

impl ParameterAttrs {
    /// Parse from `#[param(...)]` attribute args.
    pub fn parse(attr_args: &attrs::AttrArgs, field_name: &Ident) -> Result<Self> {
        let key = field_name.to_string();
        let name = attr_args.get_string("name").unwrap_or_else(|| key.clone());
        let description = attr_args.get_string("description");
        let required = attr_args.has_flag("required");
        let secret = attr_args.has_flag("secret");
        let default = attr_args.get_value("default").cloned();
        let validation = attr_args.get_string("validation");
        let options = attr_args.get_list("options");

        Ok(Self {
            key,
            name,
            description,
            required,
            secret,
            default,
            validation,
            options,
        })
    }

    /// Whether to skip this field.
    pub fn is_skip(attr_args: &attrs::AttrArgs) -> bool {
        attr_args.has_flag("skip")
    }

    /// Generate the parameter definition expression.
    pub fn param_def_expr(&self, field_type: &Type) -> Result<TokenStream2> {
        let key = &self.key;
        let name = &self.name;
        let description = &self.description;
        let required = self.required;
        let default = self.default.as_ref();

        let kind = infer_kind(field_type, self.secret);
        let (init_expr, default_setter) = match kind {
            ParamKind::Secret => (
                quote!(::nebula_parameter::types::SecretParameter::new(#key, #name)),
                default_text_setter(default),
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
            .as_ref()
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
}

fn default_text_setter(default: Option<&attrs::AttrValue>) -> TokenStream2 {
    match default {
        Some(attrs::AttrValue::Lit(Lit::Str(value))) => {
            quote!(param.default = Some(#value.to_string());)
        }
        _ => quote! {},
    }
}

fn default_number_setter(default: Option<&attrs::AttrValue>) -> TokenStream2 {
    match default {
        Some(attrs::AttrValue::Lit(Lit::Int(value))) => {
            quote!(param.default = Some((#value) as f64);)
        }
        Some(attrs::AttrValue::Lit(Lit::Float(value))) => {
            quote!(param.default = Some(#value);)
        }
        _ => quote! {},
    }
}

fn default_checkbox_setter(default: Option<&attrs::AttrValue>) -> TokenStream2 {
    match default {
        Some(attrs::AttrValue::Lit(Lit::Bool(value))) => {
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
    } else {
        ParamKind::Text
    }
}

fn unwrap_option(ty: &Type) -> &Type {
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
