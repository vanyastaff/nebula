use proc_macro2::TokenStream as TokenStream2;
use syn::{
    Attribute, Ident, Lit, Meta, Result, Token, Type,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

use crate::support::diag;

/// Parsed attribute arguments container.
#[derive(Debug, Clone)]
pub struct AttrArgs {
    pub items: Vec<AttrItem>,
}

/// A single attribute item.
#[derive(Debug, Clone)]
pub enum AttrItem {
    /// A flag like `required` or `secret`
    Flag(Ident),
    /// Key-value pair like `key = "value"`
    KeyValue { key: Ident, value: AttrValue },
    /// Nested list like `options(a, b, c)`
    List { key: Ident, values: Vec<AttrValue> },
}

/// Value within a list.
#[derive(Debug, Clone)]
pub enum AttrValue {
    Ident(Ident),
    Lit(Lit),
    Tokens(TokenStream2),
}

impl AttrArgs {
    /// Find a key-value pair by key name.
    pub fn get_value(&self, key: &str) -> Option<&AttrValue> {
        self.items.iter().find_map(|item| match item {
            AttrItem::KeyValue { key: k, value } if k == key => Some(value),
            _ => None,
        })
    }

    /// Get a string value by key.
    pub fn get_string(&self, key: &str) -> Option<String> {
        self.get_value(key).and_then(|lit| match lit {
            AttrValue::Lit(Lit::Str(s)) => Some(s.value()),
            _ => None,
        })
    }

    /// Get an integer value by key.
    pub fn get_int(&self, key: &str) -> Option<u64> {
        self.get_value(key).and_then(|lit| match lit {
            AttrValue::Lit(Lit::Int(i)) => i.base10_parse().ok(),
            _ => None,
        })
    }

    /// Parse a type from a key-value pair.
    pub fn get_type(&self, key: &str) -> Result<Option<Type>> {
        let value = match self.get_value(key) {
            Some(value) => value,
            None => return Ok(None),
        };

        let ty = match value {
            AttrValue::Lit(Lit::Str(s)) => syn::parse_str::<Type>(&s.value())
                .map_err(|e| diag::error_spanned(s, format!("invalid type for `{key}`: {e}")))?,
            AttrValue::Ident(i) => syn::parse_str::<Type>(&i.to_string())
                .map_err(|e| diag::error_spanned(i, format!("invalid type for `{key}`: {e}")))?,
            AttrValue::Tokens(tokens) => syn::parse2::<Type>(tokens.clone()).map_err(|e| {
                diag::error_spanned(tokens, format!("invalid type for `{key}`: {e}"))
            })?,
            AttrValue::Lit(other) => {
                return Err(diag::error_spanned(
                    other,
                    format!("expected a type for `{key}`"),
                ));
            }
        };

        Ok(Some(ty))
    }

    /// Check if a flag is present.
    pub fn has_flag(&self, flag: &str) -> bool {
        self.items
            .iter()
            .any(|item| matches!(item, AttrItem::Flag(f) if f == flag))
    }

    /// Get values from a list attribute like `group = ["a", "b"]`.
    pub fn get_list(&self, key: &str) -> Option<Vec<String>> {
        self.items.iter().find_map(|item| match item {
            AttrItem::List { key: k, values } if k == key => {
                let strings: Vec<String> = values
                    .iter()
                    .filter_map(|v| match v {
                        AttrValue::Lit(Lit::Str(s)) => Some(s.value()),
                        AttrValue::Ident(i) => Some(i.to_string()),
                        _ => None,
                    })
                    .collect();
                Some(strings)
            }
            _ => None,
        })
    }

    /// Require a string value, returning an error if missing.
    pub fn require_string(&self, key: &str, span: &impl quote::ToTokens) -> Result<String> {
        self.get_string(key).ok_or_else(|| {
            diag::error_spanned(
                span,
                format!("missing required attribute `{key} = \"...\"`"),
            )
        })
    }
}

/// Parse attribute like `#[param(...)]` (the whole Attribute, not only args).
pub fn parse_attr(attr: &Attribute, expected: &str) -> Result<Option<AttrArgs>> {
    if !attr.path().is_ident(expected) {
        return Ok(None);
    }

    match &attr.meta {
        Meta::Path(_) => Ok(Some(AttrArgs { items: vec![] })),
        Meta::List(list) => {
            let args = syn::parse2::<AttrArgsParser>(list.tokens.clone())?;
            Ok(Some(args.0))
        }
        Meta::NameValue(nv) => Err(diag::error_spanned(
            nv,
            format!("#[{expected}] must be #[{expected}(...)] or #[{expected}] (not name-value)"),
        )),
    }
}

/// Parse all attributes of a given type and merge them.
pub fn parse_attrs(attrs: &[Attribute], name: &str) -> Result<AttrArgs> {
    let mut result = AttrArgs { items: vec![] };

    for attr in attrs {
        if let Some(args) = parse_attr(attr, name)? {
            result.items.extend(args.items);
        }
    }

    Ok(result)
}

struct AttrArgsParser(AttrArgs);

impl Parse for AttrArgsParser {
    fn parse(input: ParseStream) -> Result<Self> {
        let items = if input.is_empty() {
            vec![]
        } else {
            Punctuated::<AttrItemParser, Token![,]>::parse_terminated(input)?
                .into_iter()
                .map(|x| x.0)
                .collect()
        };
        Ok(Self(AttrArgs { items }))
    }
}

struct AttrItemParser(AttrItem);

impl Parse for AttrItemParser {
    fn parse(input: ParseStream) -> Result<Self> {
        let key: Ident = input.parse()?;

        if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;

            if input.peek(syn::token::Bracket) {
                let content;
                syn::bracketed!(content in input);
                let values = if content.is_empty() {
                    vec![]
                } else {
                    Punctuated::<AttrValueParser, Token![,]>::parse_terminated(&content)?
                        .into_iter()
                        .map(|x| x.0)
                        .collect()
                };
                return Ok(Self(AttrItem::List { key, values }));
            }

            let value: AttrValue = input.parse::<AttrValueParser>()?.0;
            return Ok(Self(AttrItem::KeyValue { key, value }));
        }

        if input.peek(syn::token::Paren) {
            let content;
            syn::parenthesized!(content in input);

            let values = if content.is_empty() {
                vec![]
            } else {
                Punctuated::<AttrValueParser, Token![,]>::parse_terminated(&content)?
                    .into_iter()
                    .map(|x| x.0)
                    .collect()
            };

            return Ok(Self(AttrItem::List { key, values }));
        }

        Ok(Self(AttrItem::Flag(key)))
    }
}

struct AttrValueParser(AttrValue);

impl Parse for AttrValueParser {
    fn parse(input: ParseStream) -> Result<Self> {
        if input.peek(Lit) {
            return Ok(Self(AttrValue::Lit(input.parse()?)));
        }
        if input.peek(Ident) {
            return Ok(Self(AttrValue::Ident(input.parse()?)));
        }
        Ok(Self(AttrValue::Tokens(input.parse()?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    #[test]
    fn test_parse_simple_attr() {
        let tokens = quote!(key = "value", required);
        let parsed: AttrArgsParser = syn::parse2(tokens).unwrap();

        assert_eq!(parsed.0.items.len(), 2);
        assert!(parsed.0.get_string("key").is_some());
        assert!(parsed.0.has_flag("required"));
    }

    #[test]
    fn test_parse_array_attr() {
        let tokens = quote!(group = ["a", "b", "c"]);
        let parsed: AttrArgsParser = syn::parse2(tokens).unwrap();

        let list = parsed.0.get_list("group").unwrap();
        assert_eq!(list, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_parse_type_attr() {
        let tokens = quote!(config = PgConfig, instance = crate::Pool);
        let parsed: AttrArgsParser = syn::parse2(tokens).unwrap();

        assert!(parsed.0.get_type("config").unwrap().is_some());
        assert!(parsed.0.get_type("instance").unwrap().is_some());
    }
}
