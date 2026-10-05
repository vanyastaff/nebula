//! Lower the shared checked DSL in a data record's named-condition scope.

use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Expr, ExprCall, Fields, Lit, ext::IdentExt};

use crate::{attrs::SchemaStructAttrs, codec_attrs::CodecAttrs};

pub(crate) fn declarations(
    input: &DeriveInput,
    attributes: &SchemaStructAttrs,
    projection: &CodecAttrs,
) -> syn::Result<TokenStream> {
    if attributes.conditions.is_empty() {
        return Ok(TokenStream::new());
    }
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            input,
            "named conditions require a data record",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            input,
            "named conditions require named data fields",
        ));
    };
    if projection.transparent {
        return Err(syn::Error::new_spanned(
            input,
            "transparent roots cannot declare record-local conditions",
        ));
    }
    let mut keys = BTreeMap::new();
    for field in &fields.named {
        let Some(name) = &field.ident else { continue };
        let own = CodecAttrs::parse(&field.attrs)?;
        if own.skip_input || crate::attrs::FieldAttrs::from_attrs(&field.attrs)?.skip {
            continue;
        }
        let (key, _) = crate::derive_property_type::keys(name, &own, projection, false)?;
        keys.insert(name.unraw().to_string(), key);
    }
    let definitions = attributes
        .conditions
        .iter()
        .map(|(name, expression)| (name.unraw().to_string(), expression))
        .collect::<BTreeMap<_, _>>();
    let schema = crate::crate_path();
    let mut declarations = Vec::new();
    for (name, expression) in &attributes.conditions {
        let key = name.unraw().to_string();
        let mut expansion = Expansion {
            definitions: &definitions,
            keys: &keys,
            active: BTreeSet::new(),
            nodes: 0,
        };
        expansion.active.insert(key.clone());
        let rule = expansion.rule(expression, 1)?;
        declarations.push(quote! {
            let __condition = #schema::__private::validator::Condition::try_from(#rule)
                .map_err(|_| #schema::ValidationError::builder("schema.condition.invalid_grammar").build())?;
            __builder.named_condition(#key, __condition)?;
        });
    }
    Ok(quote! {
        if __builder.direction() == #schema::SchemaDirection::Input {
            #(#declarations)*
        }
    })
}

struct Expansion<'a> {
    definitions: &'a BTreeMap<String, &'a Expr>,
    keys: &'a BTreeMap<String, String>,
    active: BTreeSet<String>,
    nodes: usize,
}

impl Expansion<'_> {
    fn rule(&mut self, expression: &Expr, depth: usize) -> syn::Result<TokenStream> {
        self.nodes += 1;
        if depth > 64 || self.nodes > 1_024 {
            return Err(syn::Error::new_spanned(
                expression,
                "expanded condition exceeds depth/node budget",
            ));
        }
        let (name, call) = invocation(expression)?;
        let schema = crate::crate_path();
        if name == "condition" {
            let Expr::Path(path) = argument(call, 0)? else {
                return Err(syn::Error::new_spanned(
                    expression,
                    "expected named condition identifier",
                ));
            };
            let key = path
                .path
                .get_ident()
                .ok_or_else(|| {
                    syn::Error::new_spanned(path, "expected named condition identifier")
                })?
                .unraw()
                .to_string();
            let definition = *self
                .definitions
                .get(&key)
                .ok_or_else(|| syn::Error::new_spanned(path, "undefined named condition"))?;
            if !self.active.insert(key.clone()) {
                return Err(syn::Error::new_spanned(path, "recursive named condition"));
            }
            let result = self.rule(definition, depth + 1);
            self.active.remove(&key);
            return result;
        }
        let builder = match name.as_str() {
            "all" | "any" => {
                let children = call
                    .args
                    .iter()
                    .map(|child| self.rule(child, depth + 1))
                    .collect::<syn::Result<Vec<_>>>()?;
                let method = syn::Ident::new(&name, proc_macro2::Span::call_site());
                quote!(#schema::__private::validator::Rule::#method([#(#children),*]))
            },
            "not" => {
                let child = self.rule(argument(call, 0)?, depth + 1)?;
                quote!(#schema::__private::validator::Rule::not(#child))
            },
            _ => {
                let pointer = self.reference(argument(call, 0)?)?;
                let path = quote!(#schema::__private::validator::foundation::FieldPath::from_pointer(#pointer)
                    .map_err(|_| #schema::ValidationError::builder("schema.condition.invalid_pointer").build())?);
                let variant = match name.as_str() {
                    "eq" => "Eq",
                    "ne" => "Ne",
                    "gt" => "Gt",
                    "gte" => "Gte",
                    "lt" => "Lt",
                    "lte" => "Lte",
                    "one_of" => "In",
                    "is_true" => "IsTrue",
                    "is_false" => "IsFalse",
                    _ => {
                        return Err(syn::Error::new_spanned(
                            expression,
                            "unsupported checked condition",
                        ));
                    },
                };
                let variant = syn::Ident::new(variant, proc_macro2::Span::call_site());
                let operands = match name.as_str() {
                    "is_true" | "is_false" => quote!(#path),
                    "one_of" => {
                        let Expr::Array(values) = argument(call, 1)? else {
                            return Err(syn::Error::new_spanned(
                                expression,
                                "expected literal array",
                            ));
                        };
                        let values = values
                            .elems
                            .iter()
                            .map(|value| quote!(#schema::__private::serde_json::json!(#value)));
                        quote!(#path, ::std::vec![#(#values),*])
                    },
                    "gt" | "gte" | "lt" | "lte" => {
                        let value = argument(call, 1)?;
                        quote!(#path, #schema::__private::serde_json::json!(#value).as_number().cloned()
                            .ok_or_else(|| #schema::ValidationError::builder("schema.condition.invalid_number").build())?)
                    },
                    _ => {
                        let value = argument(call, 1)?;
                        quote!(#path, #schema::__private::serde_json::json!(#value))
                    },
                };
                quote!(#schema::__private::validator::Rule::predicate(#schema::__private::validator::Predicate::#variant(#operands)))
            },
        };
        Ok(
            quote!(#builder.map_err(|_| #schema::ValidationError::builder("schema.condition.invalid_rule").build())?),
        )
    }

    fn reference(&self, expression: &Expr) -> syn::Result<String> {
        let (name, call) = invocation(expression)?;
        match (name.as_str(), argument(call, 0)?) {
            ("field", Expr::Path(path)) => {
                let name = path
                    .path
                    .get_ident()
                    .ok_or_else(|| {
                        syn::Error::new_spanned(path, "expected local field identifier")
                    })?
                    .unraw()
                    .to_string();
                self.keys
                    .get(&name)
                    .map(|key| format!("/{key}"))
                    .ok_or_else(|| {
                        syn::Error::new_spanned(
                            path,
                            "condition references an absent inbound field",
                        )
                    })
            },
            ("root", Expr::Lit(value)) => match &value.lit {
                Lit::Str(pointer) => Ok(pointer.value()),
                _ => Err(syn::Error::new_spanned(
                    value,
                    "expected root pointer literal",
                )),
            },
            _ => Err(syn::Error::new_spanned(
                expression,
                "expected checked field or root reference",
            )),
        }
    }
}

fn invocation(expression: &Expr) -> syn::Result<(String, &ExprCall)> {
    let Expr::Call(call) = expression else {
        return Err(syn::Error::new_spanned(
            expression,
            "expected checked condition call",
        ));
    };
    let Expr::Path(path) = &*call.func else {
        return Err(syn::Error::new_spanned(
            expression,
            "expected condition function",
        ));
    };
    let name = path
        .path
        .get_ident()
        .ok_or_else(|| syn::Error::new_spanned(path, "expected checked condition function"))?;
    Ok((name.to_string(), call))
}

fn argument(call: &ExprCall, index: usize) -> syn::Result<&Expr> {
    call.args
        .iter()
        .nth(index)
        .ok_or_else(|| syn::Error::new_spanned(call, "missing checked condition operand"))
}
