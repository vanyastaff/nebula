//! Parsing of #[validate(...)] attributes

use syn::{Attribute, Expr};

/// Validation attributes for a field.
#[allow(clippy::struct_excessive_bools)] // Each bool represents a distinct validation flag
#[derive(Debug, Default, Clone)]
pub(super) struct ValidationAttrs {
    // String validators
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub exact_length: Option<usize>,
    pub email: bool,
    pub url: bool,
    pub regex: Option<String>,
    pub alphanumeric: bool,
    pub contains: Option<String>,
    pub starts_with: Option<String>,
    pub ends_with: Option<String>,

    // Text validators (new!)
    pub uuid: bool,
    pub datetime: bool,
    pub json: bool,
    pub slug: bool,
    pub hex: bool,
    pub base64: bool,

    // Numeric validators
    pub min: Option<syn::Expr>,
    pub max: Option<syn::Expr>,
    pub range_min: Option<syn::Expr>,
    pub range_max: Option<syn::Expr>,
    pub positive: bool,
    pub negative: bool,
    pub even: bool,
    pub odd: bool,

    // Collection validators
    pub min_size: Option<usize>,
    pub max_size: Option<usize>,
    pub unique: bool,
    pub non_empty: bool,

    // Logical validators
    pub required: bool,
    pub nested: bool,
    pub custom: Option<String>,

    // Meta
    pub message: Option<String>,
    pub skip: bool,

    // Universal validator expression (для новых валидаторов без изменения derive)
    /// Raw expression like "`min_length(5).and(alphanumeric())`"
    /// Это позволяет использовать ЛЮБОЙ валидатор без изменения nebula-derive!
    pub expr: Option<String>,
}

impl ValidationAttrs {
    /// Parse validation attributes from a list of attributes.
    pub(super) fn from_attributes(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut result = Self::default();

        for attr in attrs {
            if !attr.path().is_ident("validate") {
                continue;
            }

            attr.parse_nested_meta(|meta| result.parse_meta(&meta))?;
        }

        Ok(result)
    }

    #[allow(clippy::too_many_lines)] // Parses many different validation attributes
    fn parse_meta(&mut self, meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<()> {
        let path = meta.path.clone();

        // String validators with values
        if path.is_ident("min_length") {
            let value: syn::LitInt = meta.value()?.parse()?;
            self.min_length = Some(value.base10_parse()?);
            return Ok(());
        }

        if path.is_ident("max_length") {
            let value: syn::LitInt = meta.value()?.parse()?;
            self.max_length = Some(value.base10_parse()?);
            return Ok(());
        }

        if path.is_ident("exact_length") {
            let value: syn::LitInt = meta.value()?.parse()?;
            self.exact_length = Some(value.base10_parse()?);
            return Ok(());
        }

        if path.is_ident("regex") {
            let value: syn::LitStr = meta.value()?.parse()?;
            self.regex = Some(value.value());
            return Ok(());
        }

        if path.is_ident("contains") {
            let value: syn::LitStr = meta.value()?.parse()?;
            self.contains = Some(value.value());
            return Ok(());
        }

        if path.is_ident("starts_with") {
            let value: syn::LitStr = meta.value()?.parse()?;
            self.starts_with = Some(value.value());
            return Ok(());
        }

        if path.is_ident("ends_with") {
            let value: syn::LitStr = meta.value()?.parse()?;
            self.ends_with = Some(value.value());
            return Ok(());
        }

        // Numeric validators with values
        if path.is_ident("min") {
            let expr: Expr = meta.value()?.parse()?;
            self.min = Some(expr);
            return Ok(());
        }

        if path.is_ident("max") {
            let expr: Expr = meta.value()?.parse()?;
            self.max = Some(expr);
            return Ok(());
        }

        // Collection validators with values
        if path.is_ident("min_size") {
            let value: syn::LitInt = meta.value()?.parse()?;
            self.min_size = Some(value.base10_parse()?);
            return Ok(());
        }

        if path.is_ident("max_size") {
            let value: syn::LitInt = meta.value()?.parse()?;
            self.max_size = Some(value.base10_parse()?);
            return Ok(());
        }

        // Custom validators
        if path.is_ident("custom") {
            let value: syn::LitStr = meta.value()?.parse()?;
            self.custom = Some(value.value());
            return Ok(());
        }

        if path.is_ident("message") {
            let value: syn::LitStr = meta.value()?.parse()?;
            self.message = Some(value.value());
            return Ok(());
        }

        // Universal expression - позволяет использовать ЛЮБОЙ валидатор!
        if path.is_ident("expr") {
            let value: syn::LitStr = meta.value()?.parse()?;
            self.expr = Some(value.value());
            return Ok(());
        }

        // Flag-based validators (no value)
        if path.is_ident("email") {
            self.email = true;
            return Ok(());
        }

        if path.is_ident("url") {
            self.url = true;
            return Ok(());
        }

        if path.is_ident("alphanumeric") {
            self.alphanumeric = true;
            return Ok(());
        }

        // Text validators
        if path.is_ident("uuid") {
            self.uuid = true;
            return Ok(());
        }

        if path.is_ident("datetime") {
            self.datetime = true;
            return Ok(());
        }

        if path.is_ident("json") {
            self.json = true;
            return Ok(());
        }

        if path.is_ident("slug") {
            self.slug = true;
            return Ok(());
        }

        if path.is_ident("hex") {
            self.hex = true;
            return Ok(());
        }

        if path.is_ident("base64") {
            self.base64 = true;
            return Ok(());
        }

        if path.is_ident("positive") {
            self.positive = true;
            return Ok(());
        }

        if path.is_ident("negative") {
            self.negative = true;
            return Ok(());
        }

        if path.is_ident("even") {
            self.even = true;
            return Ok(());
        }

        if path.is_ident("odd") {
            self.odd = true;
            return Ok(());
        }

        if path.is_ident("unique") {
            self.unique = true;
            return Ok(());
        }

        if path.is_ident("non_empty") {
            self.non_empty = true;
            return Ok(());
        }

        if path.is_ident("required") {
            self.required = true;
            return Ok(());
        }

        if path.is_ident("nested") {
            self.nested = true;
            return Ok(());
        }

        if path.is_ident("skip") {
            self.skip = true;
            return Ok(());
        }

        // Handle range(min = N, max = M)
        if path.is_ident("range") {
            meta.parse_nested_meta(|nested| {
                if nested.path.is_ident("min") {
                    let expr: Expr = nested.value()?.parse()?;
                    self.range_min = Some(expr);
                } else if nested.path.is_ident("max") {
                    let expr: Expr = nested.value()?.parse()?;
                    self.range_max = Some(expr);
                } else {
                    return Err(nested.error("Expected 'min' or 'max' in range"));
                }
                Ok(())
            })?;
            return Ok(());
        }

        Err(meta.error(format!(
            "Unknown validation attribute: '{}'. \n\
             Supported attributes:\n\
             - String: min_length, max_length, exact_length, email, url, regex, alphanumeric, contains, starts_with, ends_with\n\
             - Text: uuid, datetime, json, slug, hex, base64\n\
             - Numeric: min, max, range, positive, negative, even, odd\n\
             - Collection: min_size, max_size, unique, non_empty\n\
             - Logical: required, nested, custom\n\
             - Meta: message, skip, expr\n\
             \n\
             Use 'expr' for any validator not listed above: #[validate(expr = \"CustomValidator::new()\")]",
            path.get_ident().map_or_else(|| format!("{path:?}"), std::string::ToString::to_string)
        )))
    }

    /// Check if any validators are specified.
    pub(super) fn has_validators(&self) -> bool {
        self.min_length.is_some()
            || self.max_length.is_some()
            || self.exact_length.is_some()
            || self.email
            || self.url
            || self.regex.is_some()
            || self.alphanumeric
            || self.contains.is_some()
            || self.starts_with.is_some()
            || self.ends_with.is_some()
            || self.uuid
            || self.datetime
            || self.json
            || self.slug
            || self.hex
            || self.base64
            || self.min.is_some()
            || self.max.is_some()
            || self.range_min.is_some()
            || self.positive
            || self.negative
            || self.even
            || self.odd
            || self.min_size.is_some()
            || self.max_size.is_some()
            || self.unique
            || self.non_empty
            || self.required
            || self.nested
            || self.custom.is_some()
            || self.expr.is_some() // Universal expression
    }
}
