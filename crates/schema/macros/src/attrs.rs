//! Attribute parsers for `#[derive(Schema)]` / `#[derive(EnumSelect)]`.
//!
//! Two namespaces are recognised today:
//!
//! - `#[param(...)]`     — UI / metadata options (label, hint, default, secret, multiline…)
//! - `#[validate(...)]`  — value rules (required, length, range, pattern, url, email)
//!
//! Struct-level `#[schema(...)]` is reserved for a future pass (no options
//! functional today) — removed from the derive's attribute list so the
//! name stays free until the implementation lands.
//!
//! The parsers are intentionally forgiving on ordering and strict on
//! semantics: unknown keys inside a namespace produce a compile error at
//! the offending token, not a silent skip.

use syn::{
    Attribute, Expr, ExprLit, Lit, LitInt, Token,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

/// Options gathered from `#[param(...)]` on a struct field or enum variant.
#[derive(Default, Debug)]
pub(crate) struct ParamAttrs {
    pub label: Option<String>,
    pub description: Option<String>,
    pub placeholder: Option<String>,
    /// Default value — stored as a typed literal so the derive can emit
    /// a correctly-typed `serde_json::Value` for the target field kind
    /// (number → `Value::Number`, bool → `Value::Bool`, string → `Value::String`).
    pub default: Option<DefaultLit>,
    pub hint: Option<String>,
    pub secret: bool,
    pub multiline: bool,
    pub no_expression: bool,
    pub expression_required: bool,
    pub group: Option<String>,
    pub skip: bool,
}

/// Typed literal carried by `#[param(default = ...)]`.
#[derive(Debug, Clone)]
pub(crate) enum DefaultLit {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

/// Options gathered from `#[validate(...)]`.
#[derive(Default, Debug)]
pub(crate) struct ValidateAttrs {
    pub required: bool,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub pattern: Option<String>,
    pub url: bool,
    pub email: bool,
}

impl ParamAttrs {
    pub fn from_attrs(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut out = Self::default();
        for attr in attrs.iter().filter(|a| a.path().is_ident("param")) {
            let entries: Punctuated<ParamEntry, Token![,]> =
                attr.parse_args_with(Punctuated::parse_terminated)?;
            for entry in entries {
                entry.apply(&mut out)?;
            }
        }
        Ok(out)
    }
}

impl ValidateAttrs {
    pub fn from_attrs(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut out = Self::default();
        for attr in attrs.iter().filter(|a| a.path().is_ident("validate")) {
            let entries: Punctuated<ValidateEntry, Token![,]> =
                attr.parse_args_with(Punctuated::parse_terminated)?;
            for entry in entries {
                entry.apply(&mut out)?;
            }
        }
        Ok(out)
    }
}

// ── Per-namespace entry enums ─────────────────────────────────────────────

enum ParamEntry {
    KeyValue { name: syn::Ident, value: Lit },
    Flag(syn::Ident),
}

impl ParamEntry {
    fn apply(self, out: &mut ParamAttrs) -> syn::Result<()> {
        match self {
            ParamEntry::KeyValue { name, value } => {
                let key = name.to_string();
                let string_lit = |lit: &Lit, field: &str| -> syn::Result<String> {
                    if let Lit::Str(s) = lit {
                        Ok(s.value())
                    } else {
                        Err(syn::Error::new(
                            name.span(),
                            format!("`{field}` expects a string literal"),
                        ))
                    }
                };
                match key.as_str() {
                    "label" => out.label = Some(string_lit(&value, "label")?),
                    "description" => out.description = Some(string_lit(&value, "description")?),
                    "placeholder" => out.placeholder = Some(string_lit(&value, "placeholder")?),
                    "default" => {
                        out.default = Some(match &value {
                            Lit::Str(s) => DefaultLit::Str(s.value()),
                            Lit::Int(i) => DefaultLit::Int(i.base10_parse::<i64>()?),
                            Lit::Float(f) => DefaultLit::Float(f.base10_parse::<f64>()?),
                            Lit::Bool(b) => DefaultLit::Bool(b.value),
                            other => {
                                return Err(syn::Error::new_spanned(
                                    other,
                                    "#[param(default = ..)] expects a string, integer, \
                                     float, or bool literal",
                                ));
                            },
                        });
                    },
                    "hint" => out.hint = Some(string_lit(&value, "hint")?),
                    "group" => out.group = Some(string_lit(&value, "group")?),
                    other => {
                        return Err(syn::Error::new(
                            name.span(),
                            format!("unknown #[param(..)] option `{other}`"),
                        ));
                    },
                }
                Ok(())
            },
            ParamEntry::Flag(name) => {
                match name.to_string().as_str() {
                    "secret" => out.secret = true,
                    "multiline" => out.multiline = true,
                    "no_expression" => out.no_expression = true,
                    "expression_required" => out.expression_required = true,
                    "skip" => out.skip = true,
                    other => {
                        return Err(syn::Error::new(
                            name.span(),
                            format!("unknown #[param(..)] flag `{other}`"),
                        ));
                    },
                }
                Ok(())
            },
        }
    }
}

impl Parse for ParamEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: syn::Ident = input.parse()?;
        if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            let value: Lit = input.parse()?;
            Ok(ParamEntry::KeyValue { name, value })
        } else {
            Ok(ParamEntry::Flag(name))
        }
    }
}

enum ValidateEntry {
    Flag(syn::Ident),
    Length {
        min: Option<usize>,
        max: Option<usize>,
    },
    Range {
        min: Option<i64>,
        max: Option<i64>,
    },
    Pattern(String),
}

impl ValidateEntry {
    fn apply(self, out: &mut ValidateAttrs) -> syn::Result<()> {
        match self {
            ValidateEntry::Flag(name) => {
                match name.to_string().as_str() {
                    "required" => out.required = true,
                    "url" => out.url = true,
                    "email" => out.email = true,
                    other => {
                        return Err(syn::Error::new(
                            name.span(),
                            format!("unknown #[validate(..)] flag `{other}`"),
                        ));
                    },
                }
                Ok(())
            },
            ValidateEntry::Length { min, max } => {
                out.min_length = min;
                out.max_length = max;
                Ok(())
            },
            ValidateEntry::Range { min, max } => {
                out.min = min;
                out.max = max;
                Ok(())
            },
            ValidateEntry::Pattern(pat) => {
                out.pattern = Some(pat);
                Ok(())
            },
        }
    }
}

impl Parse for ValidateEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: syn::Ident = input.parse()?;
        let key = name.to_string();
        if input.peek(syn::token::Paren) {
            // e.g. `length(min = 1, max = 100)` or `range(1..=300)`
            let content;
            syn::parenthesized!(content in input);
            match key.as_str() {
                "length" => parse_length(&content),
                "range" => parse_range(&content),
                other => Err(syn::Error::new(
                    name.span(),
                    format!("unknown #[validate(..)] function `{other}`"),
                )),
            }
        } else if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            let lit: Lit = input.parse()?;
            if key == "pattern" {
                if let Lit::Str(s) = &lit {
                    let _ = name;
                    Ok(ValidateEntry::Pattern(s.value()))
                } else {
                    Err(syn::Error::new(
                        name.span(),
                        "#[validate(pattern = ..)] expects a string literal",
                    ))
                }
            } else {
                Err(syn::Error::new(
                    name.span(),
                    format!("unknown #[validate(..)] option `{key}`"),
                ))
            }
        } else {
            Ok(ValidateEntry::Flag(name))
        }
    }
}

fn parse_length(input: ParseStream) -> syn::Result<ValidateEntry> {
    let span = input.span();
    let mut min = None;
    let mut max = None;
    let entries: Punctuated<LengthEntry, Token![,]> = Punctuated::parse_terminated(input)?;
    for entry in entries {
        match entry {
            LengthEntry::Min(v) => min = Some(v),
            LengthEntry::Max(v) => max = Some(v),
        }
    }
    if let (Some(min_v), Some(max_v)) = (min, max)
        && min_v > max_v
    {
        return Err(syn::Error::new(
            span,
            format!("#[validate(length(..))]: min ({min_v}) cannot exceed max ({max_v})"),
        ));
    }
    Ok(ValidateEntry::Length { min, max })
}

enum LengthEntry {
    Min(usize),
    Max(usize),
}

impl Parse for LengthEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: syn::Ident = input.parse()?;
        input.parse::<Token![=]>()?;
        let lit: LitInt = input.parse()?;
        let v: usize = lit.base10_parse()?;
        match name.to_string().as_str() {
            "min" => Ok(LengthEntry::Min(v)),
            "max" => Ok(LengthEntry::Max(v)),
            other => Err(syn::Error::new(
                name.span(),
                format!("#[validate(length(..))] key must be `min` or `max`, got `{other}`"),
            )),
        }
    }
}

fn parse_range(input: ParseStream) -> syn::Result<ValidateEntry> {
    let span = input.span();
    // Accept `min..=max`, `min..max`, or standalone ranges.
    let expr: Expr = input.parse()?;
    let (min, max) = match expr {
        Expr::Range(r) => {
            let min = match r.start.as_deref() {
                Some(start) => Some(lit_to_i64(start)?),
                None => None,
            };
            let max = match (r.end.as_deref(), r.limits) {
                (Some(end), syn::RangeLimits::Closed(_)) => Some(lit_to_i64(end)?),
                (Some(end_expr), syn::RangeLimits::HalfOpen(_)) => {
                    let end_val = lit_to_i64(end_expr)?;
                    Some(end_val.checked_sub(1).ok_or_else(|| {
                        syn::Error::new_spanned(
                            end_expr,
                            "#[validate(range(..))]: half-open upper bound underflows i64::MIN",
                        )
                    })?)
                },
                (None, _) => None,
            };
            (min, max)
        },
        other => {
            return Err(syn::Error::new_spanned(
                other,
                "#[validate(range(..))] expects a range expression",
            ));
        },
    };
    if let (Some(min_v), Some(max_v)) = (min, max)
        && min_v > max_v
    {
        return Err(syn::Error::new(
            span,
            format!("#[validate(range(..))]: min ({min_v}) cannot exceed max ({max_v})"),
        ));
    }
    Ok(ValidateEntry::Range { min, max })
}

/// Parse an integer literal bound for `#[validate(range(..))]`.
///
/// Returns a `syn::Error` anchored at the offending expression when the
/// bound is not an integer literal or does not fit in `i64`; this is
/// strictly better than the earlier `Option` signature, which silently
/// dropped invalid bounds and weakened the enforced range.
fn lit_to_i64(expr: &Expr) -> syn::Result<i64> {
    if let Expr::Lit(ExprLit {
        lit: Lit::Int(i), ..
    }) = expr
    {
        i.base10_parse::<i64>()
    } else {
        Err(syn::Error::new_spanned(
            expr,
            "#[validate(range(..))]: bounds must be integer literals",
        ))
    }
}
