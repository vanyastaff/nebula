//! Serde's directional wire projection, shared by structural and codec derives.

use proc_macro2::Span;
use syn::{Attribute, Expr, ExprLit, Lit, Meta, Token, punctuated::Punctuated, spanned::Spanned};

use crate::attrs::RenameRule;

#[derive(Default)]
pub(crate) struct CodecAttrs {
    pub input_name: Option<String>,
    pub output_name: Option<String>,
    pub input_rule: Option<RenameRule>,
    pub output_rule: Option<RenameRule>,
    pub aliases: Vec<String>,
    pub skip_input: bool,
    pub skip_output: bool,
    pub optional_output: bool,
    /// Serde's named default provider; only admitted beside a literal schema
    /// default, whose agreement the generated graph checks.
    pub default: Option<syn::Path>,
    pub transparent: bool,
    pub deny_unknown: bool,
    pub tag: Option<String>,
    pub content: Option<String>,
}

impl CodecAttrs {
    pub(crate) fn parse(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut output = Self::default();
        let projected_default = crate::attrs::FieldAttrs::from_attrs(attrs)?
            .default
            .is_some();
        let mut seen = std::collections::HashSet::new();
        for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
            for meta in attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)? {
                let key = meta
                    .path()
                    .get_ident()
                    .map(ToString::to_string)
                    .ok_or_else(|| {
                        syn::Error::new_spanned(
                            &meta,
                            "serde projection options require simple names",
                        )
                    })?;
                if key != "alias" && !seen.insert(key.clone()) {
                    return Err(syn::Error::new_spanned(
                        &meta,
                        format!("duplicate serde option `{key}`"),
                    ));
                }
                match (&*key, &meta) {
                    ("rename", Meta::NameValue(value)) => {
                        let name = string(&value.value)?;
                        output.input_name = Some(name.clone());
                        output.output_name = Some(name);
                    },
                    ("rename_all", Meta::NameValue(value)) => {
                        let rule = RenameRule::parse(&string(&value.value)?, meta.span())?;
                        output.input_rule = Some(rule);
                        output.output_rule = Some(rule);
                    },
                    ("rename" | "rename_all", Meta::List(list)) => {
                        let mut directions = std::collections::HashSet::new();
                        for entry in
                            list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?
                        {
                            let Meta::NameValue(value) = &entry else {
                                return Err(syn::Error::new_spanned(
                                    entry,
                                    "expected serialize or deserialize name",
                                ));
                            };
                            let direction = value
                                .path
                                .get_ident()
                                .map(ToString::to_string)
                                .unwrap_or_default();
                            if !matches!(&*direction, "serialize" | "deserialize")
                                || !directions.insert(direction.clone())
                            {
                                return Err(syn::Error::new_spanned(
                                    entry,
                                    "invalid or duplicate serde direction",
                                ));
                            }
                            let name = string(&value.value)?;
                            if key == "rename" {
                                if direction == "serialize" {
                                    output.output_name = Some(name);
                                } else {
                                    output.input_name = Some(name);
                                }
                            } else {
                                let rule = RenameRule::parse(&name, entry.span())?;
                                if direction == "serialize" {
                                    output.output_rule = Some(rule);
                                } else {
                                    output.input_rule = Some(rule);
                                }
                            }
                        }
                    },
                    ("alias", Meta::NameValue(value)) => output.aliases.push(string(&value.value)?),
                    ("tag", Meta::NameValue(value)) => output.tag = Some(string(&value.value)?),
                    ("content", Meta::NameValue(value)) => {
                        output.content = Some(string(&value.value)?);
                    },
                    ("skip", Meta::Path(_)) => {
                        output.skip_input = true;
                        output.skip_output = true;
                    },
                    ("skip_deserializing", Meta::Path(_)) => output.skip_input = true,
                    ("skip_serializing", Meta::Path(_)) => output.skip_output = true,
                    ("skip_serializing_if", Meta::NameValue(value)) => {
                        let predicate: syn::Path = syn::parse_str(&string(&value.value)?)?;
                        let segments = predicate
                            .segments
                            .iter()
                            .map(|segment| segment.ident.to_string())
                            .collect::<Vec<_>>();
                        if !matches!(segments.as_slice(), [option, method] if option == "Option" && method == "is_none")
                            && !matches!(segments.as_slice(), [namespace, module, option, method] if matches!(namespace.as_str(), "std" | "core") && module == "option" && option == "Option" && method == "is_none")
                        {
                            return Err(syn::Error::new_spanned(
                                value,
                                "skip_serializing_if supports only Option::is_none on an actual Option<T>; arbitrary predicates require a reviewed codec adapter",
                            ));
                        }
                        output.optional_output = true;
                    },
                    ("default", Meta::NameValue(value)) if projected_default => {
                        let mut provider: syn::Path = syn::parse_str(&string(&value.value)?)?;
                        // Errors about the provider point at the authored attribute.
                        for segment in &mut provider.segments {
                            segment.ident.set_span(value.value.span());
                        }
                        output.default = Some(provider);
                    },
                    ("default", _) => {
                        return Err(syn::Error::new_spanned(
                            &meta,
                            "serde defaults require an explicit literal projected and validated schema default; optional presence alone is not a codec witness",
                        ));
                    },
                    ("transparent", Meta::Path(_)) => output.transparent = true,
                    ("deny_unknown_fields", Meta::Path(_)) => output.deny_unknown = true,
                    // Serde's implementation-only configuration does not change wire shape.
                    ("crate" | "bound", Meta::NameValue(_) | Meta::List(_)) => {},
                    _ => {
                        return Err(syn::Error::new_spanned(
                            &meta,
                            format!(
                                "serde option `{key}` has no checked schema projection; use an explicit reviewed PropertyType/codec adapter"
                            ),
                        ));
                    },
                }
            }
        }
        if output.content.is_some() && output.tag.is_none() {
            return Err(syn::Error::new(
                Span::call_site(),
                "serde content requires a tag",
            ));
        }
        if output.tag.is_some() && output.content.is_none() {
            return Err(syn::Error::new(
                Span::call_site(),
                "internally tagged enums require a reviewed codec adapter",
            ));
        }
        Ok(output)
    }
}

/// Return the emitted domain after an owned Option::is_none omission.
/// The generated type equality check confirms an unqualified Option names the
/// standard container rather than a same-named user type.
pub(crate) fn output_present_type<'a>(
    projection: &CodecAttrs,
    ty: &'a syn::Type,
) -> syn::Result<Option<&'a syn::Type>> {
    if !projection.optional_output {
        return Ok(None);
    }
    if let syn::Type::Path(path) = ty
        && let Some(segment) = path.path.segments.last()
        && segment.ident == "Option"
        && let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments
        && arguments.args.len() == 1
        && let Some(syn::GenericArgument::Type(inner)) = arguments.args.first()
    {
        return Ok(Some(inner));
    }
    Err(syn::Error::new_spanned(
        ty,
        "Option::is_none omission requires an explicit Option<T> field type",
    ))
}

fn string(expression: &Expr) -> syn::Result<String> {
    if let Expr::Lit(ExprLit {
        lit: Lit::Str(value),
        ..
    }) = expression
    {
        Ok(value.value())
    } else {
        Err(syn::Error::new_spanned(
            expression,
            "serde option requires a string literal",
        ))
    }
}
