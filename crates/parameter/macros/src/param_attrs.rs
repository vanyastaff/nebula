//! Parsed `#[param(...)]` and `#[validate(...)]` field attributes.

use nebula_macro_support::attrs;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Ident, Result, Type};

/// Parsed parameter field attributes.
#[derive(Debug, Clone)]
pub struct ParameterAttrs {
    /// Field key (from field name).
    pub key: String,
    /// Display label.
    pub label: Option<String>,
    /// Description text.
    pub description: Option<String>,
    /// Whether the parameter is required.
    pub required: bool,
    /// Whether the parameter is secret.
    pub secret: bool,
    /// Default value expression.
    pub default: Option<attrs::AttrValue>,
    /// Placeholder text.
    pub placeholder: Option<String>,
    /// Input hint (url, email, date, etc.).
    pub hint: Option<String>,
    /// Disable expression mode.
    pub no_expression: bool,
    /// Multiline text input.
    pub multiline: bool,
    /// Field name for the `visible_when` condition.
    pub visible_when_field: Option<String>,
    /// Value for the `visible_when` condition.
    pub visible_when_value: Option<String>,
    /// Field name for the `required_when` condition.
    pub required_when_field: Option<String>,
    /// Value for the `required_when` condition.
    pub required_when_value: Option<String>,
}

impl ParameterAttrs {
    /// Parse from `#[param(...)]` attribute args.
    pub fn parse(attr_args: &attrs::AttrArgs, field_name: &Ident) -> Result<Self> {
        let key = field_name.to_string();
        let label = attr_args.get_string("label");
        let description = attr_args.get_string("description");
        let required = attr_args.has_flag("required");
        let secret = attr_args.has_flag("secret");
        let default = attr_args.get_value("default").cloned();
        let placeholder = attr_args.get_string("placeholder");
        let hint = attr_args.get_string("hint");
        let no_expression = attr_args.has_flag("no_expression");
        let multiline = attr_args.has_flag("multiline");
        let visible_when_field = attr_args.get_string("visible_when_field");
        let visible_when_value = attr_args.get_string("visible_when_value");
        let required_when_field = attr_args.get_string("required_when_field");
        let required_when_value = attr_args.get_string("required_when_value");

        Ok(Self {
            key,
            label,
            description,
            required,
            secret,
            default,
            placeholder,
            hint,
            no_expression,
            multiline,
            visible_when_field,
            visible_when_value,
            required_when_field,
            required_when_value,
        })
    }

    /// Whether to skip this field.
    pub fn is_skip(attr_args: &attrs::AttrArgs) -> bool {
        attr_args.has_flag("skip")
    }

    /// Generate the parameter definition expression for a given Rust type.
    pub fn param_def_expr(&self, field_type: &Type) -> Result<TokenStream2> {
        // Determine if Optional
        let (inner_type, is_optional) = unwrap_option(field_type);

        // Generate the constructor based on the inner type
        let constructor = self.type_to_constructor(inner_type);

        // Shared attribute setters
        let label_setter = self
            .label
            .as_ref()
            .map(|v| quote!(.label(#v)))
            .unwrap_or_default();

        let description_setter = self
            .description
            .as_ref()
            .map(|v| quote!(.description(#v)))
            .unwrap_or_default();

        let placeholder_setter = self
            .placeholder
            .as_ref()
            .map(|v| quote!(.placeholder(#v)))
            .unwrap_or_default();

        let required_setter = if self.required && !is_optional {
            quote!(.required())
        } else {
            quote!()
        };

        let secret_setter = if self.secret {
            quote!(.secret())
        } else {
            quote!()
        };

        let no_expr_setter = if self.no_expression {
            quote!(.no_expression())
        } else {
            quote!()
        };

        let default_setter = self
            .default
            .as_ref()
            .map(|v| {
                let val = attr_value_to_json(v);
                quote!(.default(#val))
            })
            .unwrap_or_default();

        let visible_setter = match (&self.visible_when_field, &self.visible_when_value) {
            (Some(field), Some(value)) => quote! {
                .visible_when(::nebula_parameter::conditions::Condition::eq(#field, #value))
            },
            _ => quote! {},
        };

        let required_when_setter = match (&self.required_when_field, &self.required_when_value) {
            (Some(field), Some(value)) => quote! {
                .required_when(::nebula_parameter::conditions::Condition::eq(#field, #value))
            },
            _ => quote! {},
        };

        Ok(quote! {
            #constructor
                #label_setter
                #description_setter
                #placeholder_setter
                #required_setter
                #secret_setter
                #no_expr_setter
                #default_setter
                #visible_setter
                #required_when_setter
        })
    }

    /// Map a Rust type to a Parameter constructor call.
    fn type_to_constructor(&self, ty: &Type) -> TokenStream2 {
        let key = &self.key;

        // Check for Vec<T> → list
        if let Some(inner) = unwrap_vec(ty) {
            // Vec<T> where T might be a nested struct
            let item_constructor = self.inner_type_constructor("_item", inner);
            return quote! {
                ::nebula_parameter::parameter::Parameter::list(#key, #item_constructor)
            };
        }

        // Match on type path segments
        let type_name = type_to_string(ty);

        match type_name.as_str() {
            "String" | "str" => {
                if self.multiline {
                    quote! {
                        ::nebula_parameter::parameter::Parameter::string(#key).multiline()
                    }
                } else if let Some(hint) = &self.hint {
                    match hint.as_str() {
                        "url" => quote! {
                            ::nebula_parameter::parameter::Parameter::string(#key)
                                .input_hint(::nebula_parameter::InputHint::Url)
                        },
                        "email" => quote! {
                            ::nebula_parameter::parameter::Parameter::string(#key)
                                .input_hint(::nebula_parameter::InputHint::Email)
                        },
                        "date" => quote! { ::nebula_parameter::parameter::Parameter::date(#key) },
                        "datetime" => {
                            quote! { ::nebula_parameter::parameter::Parameter::datetime(#key) }
                        },
                        "time" => quote! { ::nebula_parameter::parameter::Parameter::time(#key) },
                        "color" => quote! { ::nebula_parameter::parameter::Parameter::color(#key) },
                        "password" => quote! {
                            ::nebula_parameter::parameter::Parameter::string(#key)
                                .input_hint(::nebula_parameter::InputHint::Password)
                        },
                        "phone" => quote! {
                            ::nebula_parameter::parameter::Parameter::string(#key)
                                .input_hint(::nebula_parameter::InputHint::Phone)
                        },
                        "ip" => quote! {
                            ::nebula_parameter::parameter::Parameter::string(#key)
                                .input_hint(::nebula_parameter::InputHint::Ip)
                        },
                        // Unknown hint — fall back to plain string
                        _ => quote! {
                            ::nebula_parameter::parameter::Parameter::string(#key)
                        },
                    }
                } else {
                    quote! { ::nebula_parameter::parameter::Parameter::string(#key) }
                }
            },
            "bool" => {
                quote! { ::nebula_parameter::parameter::Parameter::boolean(#key) }
            },
            "u8" | "u16" | "u32" | "u64" | "usize" | "i8" | "i16" | "i32" | "i64" | "isize" => {
                quote! { ::nebula_parameter::parameter::Parameter::integer(#key) }
            },
            "f32" | "f64" => {
                quote! { ::nebula_parameter::parameter::Parameter::number(#key) }
            },
            // For any other type: assume it implements HasParameters (nested object)
            // or HasSelectOptions (enum select). We try HasSelectOptions first
            // via trait resolution — if it fails, fall back to HasParameters.
            _ => {
                let ty_ident = ty;
                quote! {{
                    // Try to use HasSelectOptions if available, otherwise HasParameters
                    // This uses a trait-based approach — the compiler resolves which.
                    <#ty_ident as ::nebula_parameter::InferParameterType>::into_parameter(#key)
                }}
            },
        }
    }

    /// Generate constructor for a list item's inner type.
    fn inner_type_constructor(&self, id: &str, ty: &Type) -> TokenStream2 {
        let type_name = type_to_string(ty);
        match type_name.as_str() {
            "String" | "str" => quote! { ::nebula_parameter::parameter::Parameter::string(#id) },
            "bool" => quote! { ::nebula_parameter::parameter::Parameter::boolean(#id) },
            "u8" | "u16" | "u32" | "u64" | "usize" | "i8" | "i16" | "i32" | "i64" | "isize" => {
                quote! { ::nebula_parameter::parameter::Parameter::integer(#id) }
            },
            "f32" | "f64" => quote! { ::nebula_parameter::parameter::Parameter::number(#id) },
            _ => {
                let ty_ident = ty;
                quote! {{
                    <#ty_ident as ::nebula_parameter::InferParameterType>::into_parameter(#id)
                }}
            },
        }
    }
}

/// Parsed `#[validate(...)]` field attributes.
#[derive(Debug, Clone, Default)]
pub struct ValidateAttrs {
    /// Mark field as required.
    pub required: bool,
    /// Apply URL validation rule.
    pub url: bool,
    /// Apply email validation rule.
    pub email: bool,
    /// Apply minimum length rule.
    pub min_length: Option<u64>,
    /// Apply maximum length rule.
    pub max_length: Option<u64>,
    /// Apply minimum value rule.
    pub min: Option<u64>,
    /// Apply maximum value rule.
    pub max: Option<u64>,
    /// Apply pattern (regex) rule.
    pub pattern: Option<String>,
}

impl ValidateAttrs {
    /// Parse from `#[validate(...)]` attribute args.
    pub fn parse(attr_args: &attrs::AttrArgs) -> Result<Self> {
        Ok(Self {
            required: attr_args.has_flag("required"),
            url: attr_args.has_flag("url"),
            email: attr_args.has_flag("email"),
            min_length: attr_args.get_int("min_length"),
            max_length: attr_args.get_int("max_length"),
            min: attr_args.get_int("min"),
            max: attr_args.get_int("max"),
            pattern: attr_args.get_string("pattern"),
        })
    }

    /// Generate chained `.with_rule(...)` expressions for all active rules.
    pub fn rule_exprs(&self) -> Vec<TokenStream2> {
        let mut rules = Vec::new();
        if self.url {
            rules.push(
                quote! { .with_rule(::nebula_parameter::rules::Rule::Url { message: None }) },
            );
        }
        if self.email {
            rules.push(
                quote! { .with_rule(::nebula_parameter::rules::Rule::Email { message: None }) },
            );
        }
        if let Some(min) = self.min_length {
            let min = min as usize;
            rules.push(
                quote! { .with_rule(::nebula_parameter::rules::Rule::MinLength { min: #min, message: None }) },
            );
        }
        if let Some(max) = self.max_length {
            let max = max as usize;
            rules.push(
                quote! { .with_rule(::nebula_parameter::rules::Rule::MaxLength { max: #max, message: None }) },
            );
        }
        if let Some(min) = self.min {
            rules.push(
                quote! { .with_rule(::nebula_parameter::rules::Rule::Min { min: ::serde_json::Number::from(#min), message: None }) },
            );
        }
        if let Some(max) = self.max {
            rules.push(
                quote! { .with_rule(::nebula_parameter::rules::Rule::Max { max: ::serde_json::Number::from(#max), message: None }) },
            );
        }
        if let Some(pat) = &self.pattern {
            rules.push(
                quote! { .with_rule(::nebula_parameter::rules::Rule::Pattern { pattern: #pat.to_owned(), message: None }) },
            );
        }
        rules
    }
}

/// Unwrap `Option<T>` → (T, true). If not Option, return (ty, false).
fn unwrap_option(ty: &Type) -> (&Type, bool) {
    let Type::Path(type_path) = ty else {
        return (ty, false);
    };
    let Some(segment) = type_path.path.segments.last() else {
        return (ty, false);
    };
    if segment.ident != "Option" {
        return (ty, false);
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return (ty, false);
    };
    match args.args.first() {
        Some(syn::GenericArgument::Type(inner)) => (inner, true),
        _ => (ty, false),
    }
}

/// Unwrap `Vec<T>` → Some(T). If not Vec, return None.
fn unwrap_vec(ty: &Type) -> Option<&Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident != "Vec" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    match args.args.first() {
        Some(syn::GenericArgument::Type(inner)) => Some(inner),
        _ => None,
    }
}

/// Extract the last path segment as a string for type matching.
pub(crate) fn type_to_string(ty: &Type) -> String {
    let Type::Path(type_path) = ty else {
        return String::new();
    };
    type_path
        .path
        .segments
        .last()
        .map(|s| s.ident.to_string())
        .unwrap_or_default()
}

/// Convert an `AttrValue` to a `serde_json::json!()` expression.
fn attr_value_to_json(val: &attrs::AttrValue) -> TokenStream2 {
    match val {
        attrs::AttrValue::Lit(lit) => match lit {
            syn::Lit::Str(s) => {
                let v = s.value();
                quote! { ::serde_json::json!(#v) }
            },
            syn::Lit::Int(i) => {
                let v: i64 = i.base10_parse().unwrap_or(0);
                quote! { ::serde_json::json!(#v) }
            },
            syn::Lit::Float(f) => {
                let v: f64 = f.base10_parse().unwrap_or(0.0);
                quote! { ::serde_json::json!(#v) }
            },
            syn::Lit::Bool(b) => {
                let v = b.value;
                quote! { ::serde_json::json!(#v) }
            },
            _ => quote! { ::serde_json::Value::Null },
        },
        attrs::AttrValue::Ident(i) => {
            let s = i.to_string();
            quote! { ::serde_json::json!(#s) }
        },
        attrs::AttrValue::Tokens(ts) => {
            quote! { ::serde_json::json!(#ts) }
        },
    }
}
