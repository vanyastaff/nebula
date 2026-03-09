//! Parsed `#[param(...)]` field attributes.
//!
//! [`ParameterAttrs`] holds per-field parameter metadata for the Parameters derive.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Ident, Result};

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
    pub fn param_def_expr(&self) -> Result<TokenStream2> {
        let key = &self.key;
        let name = &self.name;
        let description = &self.description;
        let required = self.required;
        let secret = self.secret;

        let description_setter = description
            .as_ref()
            .map(|value| quote!(.with_description(#value)))
            .unwrap_or_default();

        let required_setter = if required { quote!(.required()) } else { quote!() };
        let secret_setter = if secret { quote!(.secret()) } else { quote!() };

        Ok(quote! {
            ::nebula_parameter::schema::Field::text(#key)
                .with_label(#name)
                #description_setter
                #required_setter
                #secret_setter
        })
    }
}
