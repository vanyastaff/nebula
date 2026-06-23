//! Attribute parsers for `#[derive(Schema)]` / `#[derive(EnumSelect)]`.
//!
//! Two namespaces are recognised today:
//!
//! - `#[field(...)]` — UI / metadata options (label, hint, default, secret, multiline,
//!   `enum_select`, …)
//! - `#[validate(...)]`  — value rules (required, length, range, pattern, url, email)
//!
//! Struct-level `#[schema(...)]` on `#[derive(Schema)]` supports:
//!
//! - `custom = "..."` — emits a deferred `Rule::custom` on the built schema (wire-level expression
//!   string; engine evaluation is Phase 3+).
//!
//! The parsers are intentionally forgiving on ordering and strict on
//! semantics: unknown keys inside a namespace produce a compile error at
//! the offending token, not a silent skip.

use proc_macro2::Span;
use syn::{
    Attribute, Expr, ExprLit, Lit, LitInt, LitStr, Meta, Token,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    spanned::Spanned,
};

/// Options gathered from `#[field(...)]` on a struct field or enum variant.
#[derive(Default, Debug)]
pub(crate) struct FieldAttrs {
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
    /// When true, a user-defined field type is emitted as a static `Select` field whose options
    /// come from `HasSelectOptions` (typically `#[derive(EnumSelect)]` on an enum).
    pub enum_select: bool,
    pub group: Option<String>,
    pub skip: bool,
}

/// Typed literal carried by `#[field(default = ...)]`.
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

impl FieldAttrs {
    pub(crate) fn from_attrs(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut out = Self::default();
        for attr in attrs.iter().filter(|a| a.path().is_ident("field")) {
            let entries: Punctuated<FieldEntry, Token![,]> =
                attr.parse_args_with(Punctuated::parse_terminated)?;
            for entry in entries {
                entry.apply(&mut out)?;
            }
        }
        Ok(out)
    }
}

impl ValidateAttrs {
    pub(crate) fn from_attrs(attrs: &[Attribute]) -> syn::Result<Self> {
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

enum FieldEntry {
    KeyValue { name: syn::Ident, value: Lit },
    Flag(syn::Ident),
}

impl FieldEntry {
    fn apply(self, out: &mut FieldAttrs) -> syn::Result<()> {
        match self {
            FieldEntry::KeyValue { name, value } => {
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
                                    "#[field(default = ..)] expects a string, integer, \
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
                            format!("unknown #[field(..)] option `{other}`"),
                        ));
                    },
                }
                Ok(())
            },
            FieldEntry::Flag(name) => {
                match name.to_string().as_str() {
                    "secret" => out.secret = true,
                    "multiline" => out.multiline = true,
                    "no_expression" => out.no_expression = true,
                    "expression_required" => out.expression_required = true,
                    "enum_select" => out.enum_select = true,
                    "skip" => out.skip = true,
                    other => {
                        return Err(syn::Error::new(
                            name.span(),
                            format!("unknown #[field(..)] flag `{other}`"),
                        ));
                    },
                }
                Ok(())
            },
        }
    }
}

impl Parse for FieldEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: syn::Ident = input.parse()?;
        if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            let value: Lit = input.parse()?;
            Ok(FieldEntry::KeyValue { name, value })
        } else {
            Ok(FieldEntry::Flag(name))
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

// ── Struct-level #[schema(...)] on #[derive(Schema)] ─────────────────────────

/// Options gathered from `#[schema(...)]` on the derive target struct.
#[derive(Default, Debug)]
pub(crate) struct SchemaStructAttrs {
    /// Wire-level `Rule::Deferred(DeferredRule::Custom(..))` expression strings.
    pub custom: Vec<LitStr>,
    /// Field keys reserved against reuse (`#[schema(reserved("old_key"))]`). A
    /// reserved key may not be used by any field of this struct — the derive
    /// rejects a collision at expansion. Kept as `LitStr` so the span points at
    /// the offending literal in diagnostics.
    pub reserved: Vec<LitStr>,
}

impl SchemaStructAttrs {
    pub(crate) fn from_attrs(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut out = Self::default();
        for attr in attrs.iter().filter(|a| a.path().is_ident("schema")) {
            let entries: Punctuated<SchemaEntry, Token![,]> =
                attr.parse_args_with(Punctuated::parse_terminated)?;
            for entry in entries {
                entry.apply(&mut out)?;
            }
        }
        Ok(out)
    }
}

enum SchemaEntry {
    Custom { value: LitStr },
    Reserved { keys: Vec<LitStr> },
}

impl SchemaEntry {
    fn apply(self, out: &mut SchemaStructAttrs) -> syn::Result<()> {
        match self {
            Self::Custom { value } => {
                out.custom.push(value);
                Ok(())
            },
            Self::Reserved { keys } => {
                out.reserved.extend(keys);
                Ok(())
            },
        }
    }
}

impl Parse for SchemaEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: syn::Ident = input.parse()?;

        // List-form options: `reserved("a", "b")`.
        if input.peek(syn::token::Paren) {
            let content;
            syn::parenthesized!(content in input);
            let keys: Punctuated<LitStr, Token![,]> =
                content.parse_terminated(<LitStr as Parse>::parse, Token![,])?;
            return match name.to_string().as_str() {
                "reserved" => Ok(Self::Reserved {
                    keys: keys.into_iter().collect(),
                }),
                other => Err(syn::Error::new(
                    name.span(),
                    format!("unknown list-form #[schema(..)] option `{other}`"),
                )),
            };
        }

        // Assignment-form options: `custom = "..."`.
        if !input.peek(Token![=]) {
            return Err(syn::Error::new(
                name.span(),
                "expected `#[schema(custom = \"...\")]` or `#[schema(reserved(\"...\"))]`",
            ));
        }
        input.parse::<Token![=]>()?;
        let value: LitStr = input.parse()?;
        match name.to_string().as_str() {
            "custom" => Ok(Self::Custom { value }),
            other => Err(syn::Error::new(
                name.span(),
                format!("unknown #[schema(..)] option `{other}`"),
            )),
        }
    }
}

// ── serde-attribute alignment (#[serde(rename / rename_all / skip / flatten)]) ──
//
// The schema key MUST equal the serde wire key, otherwise the validator checks a
// field the deserializer never produces. The derives read the relevant
// `#[serde(...)]` attributes directly. `rename_all` reproduces serde_derive's own
// case algorithm EXACTLY — `apply_to_field` for struct fields and the separate,
// naive `apply_to_variant` for enum variants (the two genuinely differ: serde's
// `snake_case` for a variant inserts `_` before every capital, so `HTTPProxy`
// becomes `h_t_t_p_proxy`, not `http_proxy`). Only an exact copy round-trips; a
// round-trip invariant test pins this. Unsupported rules are a compile error,
// never a silent guess.

/// serde `rename_all` case rule, restricted to serde's documented set.
#[derive(Clone, Copy, Debug)]
pub(crate) enum RenameRule {
    Lower,
    Upper,
    Pascal,
    Camel,
    Snake,
    ScreamingSnake,
    Kebab,
    ScreamingKebab,
}

impl RenameRule {
    fn parse(value: &str, span: Span) -> syn::Result<Self> {
        Ok(match value {
            "lowercase" => Self::Lower,
            "UPPERCASE" => Self::Upper,
            "PascalCase" => Self::Pascal,
            "camelCase" => Self::Camel,
            "snake_case" => Self::Snake,
            "SCREAMING_SNAKE_CASE" => Self::ScreamingSnake,
            "kebab-case" => Self::Kebab,
            "SCREAMING-KEBAB-CASE" => Self::ScreamingKebab,
            other => {
                return Err(syn::Error::new(
                    span,
                    format!(
                        "#[serde(rename_all = \"{other}\")] is not supported by the schema derive; \
                         supported: lowercase, UPPERCASE, PascalCase, camelCase, snake_case, \
                         SCREAMING_SNAKE_CASE, kebab-case, SCREAMING-KEBAB-CASE"
                    ),
                ));
            },
        })
    }

    /// Apply the rule to a struct **field** name, exactly as serde_derive does
    /// (`serde_derive_internals::case::RenameRule::apply_to_field`). The caller
    /// strips any raw-ident prefix first.
    pub(crate) fn apply_to_field(self, field: &str) -> String {
        match self {
            // Fields are already lowercase snake_case, so serde leaves these as-is.
            Self::Lower | Self::Snake => field.to_owned(),
            Self::Upper | Self::ScreamingSnake => field.to_ascii_uppercase(),
            Self::Pascal => {
                let mut pascal = String::new();
                let mut capitalize = true;
                for ch in field.chars() {
                    if ch == '_' {
                        capitalize = true;
                    } else if capitalize {
                        pascal.push(ch.to_ascii_uppercase());
                        capitalize = false;
                    } else {
                        pascal.push(ch);
                    }
                }
                pascal
            },
            Self::Camel => lower_first(&Self::Pascal.apply_to_field(field)),
            Self::Kebab => field.replace('_', "-"),
            Self::ScreamingKebab => Self::ScreamingSnake.apply_to_field(field).replace('_', "-"),
        }
    }

    /// Apply the rule to an enum **variant** name, exactly as serde_derive does
    /// (`serde_derive_internals::case::RenameRule::apply_to_variant`). This is NOT
    /// the same as [`Self::apply_to_field`]: serde's `snake_case` for a variant
    /// inserts an underscore before *every* uppercase letter (no acronym
    /// grouping), so `HTTPProxy` becomes `h_t_t_p_proxy`. The caller strips any
    /// raw-ident prefix first.
    pub(crate) fn apply_to_variant(self, variant: &str) -> String {
        match self {
            Self::Pascal => variant.to_owned(),
            Self::Lower => variant.to_ascii_lowercase(),
            Self::Upper => variant.to_ascii_uppercase(),
            Self::Camel => lower_first(variant),
            Self::Snake => {
                let mut snake = String::new();
                for (i, ch) in variant.char_indices() {
                    if i > 0 && ch.is_uppercase() {
                        snake.push('_');
                    }
                    snake.push(ch.to_ascii_lowercase());
                }
                snake
            },
            Self::ScreamingSnake => Self::Snake.apply_to_variant(variant).to_ascii_uppercase(),
            Self::Kebab => Self::Snake.apply_to_variant(variant).replace('_', "-"),
            Self::ScreamingKebab => Self::ScreamingSnake
                .apply_to_variant(variant)
                .replace('_', "-"),
        }
    }
}

/// Lowercase only the first character (`UserName` → `userName`), char-boundary
/// safe — mirrors serde's `camelCase` first-letter lowering without its byte
/// slicing (which would panic on a non-ASCII leading char).
fn lower_first(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_ascii_lowercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// The subset of `#[serde(...)]` attributes that affect the schema key, read from
/// a container (`rename_all`) or a field / variant (`rename`, `skip`, `flatten`).
#[derive(Default)]
pub(crate) struct SerdeAttrs {
    pub rename_all: Option<RenameRule>,
    pub rename: Option<String>,
    pub skip: bool,
    /// `Some(span)` when `#[serde(flatten)]` is present — used to anchor the
    /// "flatten not yet supported" compile error at the attribute.
    pub flatten_span: Option<Span>,
}

impl SerdeAttrs {
    pub(crate) fn from_attrs(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut out = Self::default();
        for attr in attrs.iter().filter(|a| a.path().is_ident("serde")) {
            let metas: Punctuated<Meta, Token![,]> =
                attr.parse_args_with(Punctuated::parse_terminated)?;
            for meta in &metas {
                match meta {
                    Meta::NameValue(nv) if nv.path.is_ident("rename_all") => {
                        out.rename_all = Some(RenameRule::parse(
                            &expr_str(&nv.value, "rename_all")?,
                            nv.span(),
                        )?);
                    },
                    Meta::NameValue(nv) if nv.path.is_ident("rename") => {
                        out.rename = Some(expr_str(&nv.value, "rename")?);
                    },
                    Meta::Path(p) if p.is_ident("skip") || p.is_ident("skip_deserializing") => {
                        out.skip = true;
                    },
                    Meta::Path(p) if p.is_ident("flatten") => {
                        out.flatten_span = Some(p.span());
                    },
                    Meta::List(l) if l.path.is_ident("rename") => {
                        return Err(syn::Error::new_spanned(
                            l,
                            "#[serde(rename(serialize = .., deserialize = ..))] split names are not \
                             yet honored by the schema derive; use a single `#[serde(rename = \"..\")]`",
                        ));
                    },
                    Meta::List(l) if l.path.is_ident("rename_all") => {
                        return Err(syn::Error::new_spanned(
                            l,
                            "#[serde(rename_all(serialize = .., deserialize = ..))] split rules are \
                             not yet honored by the schema derive; use a single \
                             `#[serde(rename_all = \"..\")]`",
                        ));
                    },
                    // Every other serde attribute is irrelevant to the schema key.
                    _ => {},
                }
            }
        }
        Ok(out)
    }
}

/// Extract a string literal from a `name = "value"` serde meta.
fn expr_str(value: &Expr, attr_name: &str) -> syn::Result<String> {
    if let Expr::Lit(ExprLit {
        lit: Lit::Str(s), ..
    }) = value
    {
        Ok(s.value())
    } else {
        Err(syn::Error::new_spanned(
            value,
            format!("#[serde({attr_name} = ..)] expects a string literal"),
        ))
    }
}
