//! Attribute parsing utilities for proc-macro derives.

use proc_macro2::TokenStream as TokenStream2;
use syn::{
    Attribute, Ident, Lit, Meta, Result, Token, Type,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

use crate::diag;

/// Parsed attribute arguments container.
#[derive(Debug, Clone)]
pub struct AttrArgs {
    /// The parsed attribute items.
    pub items: Vec<AttrItem>,
}

/// A single attribute item.
#[derive(Debug, Clone)]
pub enum AttrItem {
    /// A flag like `required` or `secret`
    Flag(Ident),
    /// Key-value pair like `key = "value"`
    KeyValue {
        /// The key identifier.
        key: Ident,
        /// The value.
        value: AttrValue,
    },
    /// Nested list like `options(a, b, c)`
    List {
        /// The key identifier.
        key: Ident,
        /// The list values.
        values: Vec<AttrValue>,
    },
}

/// Value within a list.
#[derive(Debug, Clone)]
pub enum AttrValue {
    /// An identifier value.
    Ident(Ident),
    /// A literal value.
    Lit(Lit),
    /// A token stream value.
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
            },
        };

        Ok(Some(ty))
    }

    /// Check if a flag is present.
    pub fn has_flag(&self, flag: &str) -> bool {
        self.items
            .iter()
            .any(|item| matches!(item, AttrItem::Flag(f) if f == flag))
    }

    /// Get an ident value by key (e.g. `auth_style = PostBody`).
    #[allow(dead_code)] // Reason: reserved for OAuth2/LDAP credential derive macros
    pub fn get_ident(&self, key: &str) -> Option<&Ident> {
        self.get_value(key).and_then(|v| match v {
            AttrValue::Ident(i) => Some(i),
            _ => None,
        })
    }

    /// Get an ident value as a string (e.g. `auth_style = PostBody` -> `"PostBody"`).
    #[allow(dead_code)] // Reason: reserved for OAuth2/LDAP credential derive macros
    pub fn get_ident_str(&self, key: &str) -> Option<String> {
        self.get_ident(key).map(ToString::to_string)
    }

    /// Get a boolean value by key (e.g. `pkce = true`).
    #[allow(dead_code)] // Reason: reserved for OAuth2/LDAP credential derive macros
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get_value(key).and_then(|v| match v {
            AttrValue::Lit(Lit::Bool(b)) => Some(b.value()),
            AttrValue::Ident(i) => match i.to_string().as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            },
            _ => None,
        })
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
            },
            _ => None,
        })
    }

    /// Get raw values from a list attribute like `each(email, min_length = 3)`.
    pub fn get_list_values(&self, key: &str) -> Option<&[AttrValue]> {
        self.items.iter().find_map(|item| match item {
            AttrItem::List { key: k, values } if k == key => Some(values.as_slice()),
            _ => None,
        })
    }

    /// Get type from attribute, returning None when value is a string literal.
    /// Use for credential/resource where `key = "string"` should be ignored.
    pub fn get_type_skip_string(&self, key: &str) -> Result<Option<Type>> {
        let value = match self.get_value(key) {
            Some(v) => v,
            None => return Ok(None),
        };
        if matches!(value, AttrValue::Lit(Lit::Str(_))) {
            return Ok(None);
        }
        self.get_type(key)
    }

    /// Get a list of types from an attribute like `resources = [PostgresDb, RedisCache]`.
    pub fn get_type_list(&self, key: &str) -> Result<Vec<Type>> {
        let values = self.items.iter().find_map(|item| match item {
            AttrItem::List { key: k, values } if k == key => Some(values),
            _ => None,
        });

        let Some(values) = values else {
            return Ok(Vec::new());
        };

        let mut types = Vec::with_capacity(values.len());
        for v in values {
            let ty = match v {
                AttrValue::Lit(Lit::Str(s)) => syn::parse_str::<Type>(&s.value())
                    .map_err(|e| diag::error_spanned(s, format!("invalid type: {e}")))?,
                AttrValue::Ident(i) => syn::parse_str::<Type>(&i.to_string())
                    .map_err(|e| diag::error_spanned(i, format!("invalid type: {e}")))?,
                AttrValue::Tokens(ts) => syn::parse2::<Type>(ts.clone())
                    .map_err(|e| diag::error_spanned(ts, format!("invalid type: {e}")))?,
                AttrValue::Lit(other) => {
                    return Err(diag::error_spanned(
                        other,
                        format!("expected a type in `{key}` list"),
                    ));
                },
            };
            types.push(ty);
        }
        Ok(types)
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
        },
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
        if input.peek(Ident) {
            let fork = input.fork();
            let _: Ident = fork.parse()?;
            if fork.peek(Token![=]) {
                let assign: syn::ExprAssign = input.parse()?;
                return Ok(Self(AttrValue::Tokens(quote::quote!(#assign))));
            }

            // Support call-like values inside lists, e.g. each(any(v1, v2)).
            // Without this, `any` would be parsed as a bare identifier and the
            // following parentheses would fail outer list parsing.
            if fork.peek(syn::token::Paren) {
                let call: syn::ExprCall = input.parse()?;
                return Ok(Self(AttrValue::Tokens(quote::quote!(#call))));
            }
        }

        if input.peek(Lit) {
            return Ok(Self(AttrValue::Lit(input.parse()?)));
        }
        // Try to parse a path (e.g. `protocols::OAuth2Protocol` or plain `Ident`).
        // We use syn::Path which handles `a::b::c` correctly, then decide based on
        // whether it is a single-segment plain identifier or a multi-segment path.
        if input.peek(Ident) {
            let path: syn::Path = input.parse()?;
            if path.segments.len() == 1 && path.leading_colon.is_none() {
                // Simple ident — preserve the original AttrValue::Ident behaviour.
                let ident = path.segments.into_iter().next().unwrap().ident;
                return Ok(Self(AttrValue::Ident(ident)));
            }
            // Multi-segment path — store as Tokens so get_type() can parse it as a Type.
            use quote::ToTokens as _;
            let mut ts = TokenStream2::new();
            path.to_tokens(&mut ts);
            return Ok(Self(AttrValue::Tokens(ts)));
        }
        Ok(Self(AttrValue::Tokens(input.parse()?)))
    }
}

#[cfg(test)]
mod tests {
    use quote::quote;

    use super::*;

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

    #[test]
    fn test_parse_call_value_in_list() {
        let tokens = quote!(each(any(v1, v2), required));
        let parsed: AttrArgsParser = syn::parse2(tokens).unwrap();

        let each_values = parsed.0.get_list_values("each").unwrap();
        assert_eq!(each_values.len(), 2);
        assert!(matches!(each_values[0], AttrValue::Tokens(_)));
        assert!(matches!(each_values[1], AttrValue::Ident(_)));
    }
}
