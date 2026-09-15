//! Attribute parsers for `#[derive(Schema)]` / `#[derive(EnumSelect)]`.
//!
//! Three namespaces are recognised today:
//!
//! - `#[property(...)]` — structured Phase-5 property sections
//! - `#[field(...)]` — UI / metadata options (label, hint, default, secret, multiline,
//!   `enum_select`, …); retained for existing in-workspace declarations
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

use std::collections::HashSet;

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
    pub hidden: bool,
    pub widget: Option<PropertyWidget>,
    pub no_expression: bool,
    pub expression_required: bool,
    pub expressions: Option<PropertyExpressionMode>,
    /// When true, a user-defined field type is emitted as a static `Select` field whose options
    /// come from `HasSelectOptions` (typically `#[derive(EnumSelect)]` on an enum).
    pub enum_select: bool,
    pub group: Option<String>,
    pub skip: bool,
    /// `#[field(emit_as = "..")]` — the key this field is emitted under on
    /// projection output (`to_wire_json`). Read-aliases come from `#[serde(alias)]`
    /// instead (so serde and the schema stay in sync on accepted input keys).
    pub emit_as: Option<String>,
}

/// Widget tokens accepted by `#[property(display(widget = ...))]`.
#[derive(Debug, Clone, Copy)]
pub(crate) enum PropertyWidget {
    Auto,
    Text,
    Textarea,
    Password,
    Number,
    Checkbox,
    Select,
    Radio,
    Object,
    List,
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
    pub min_items: Option<u32>,
    pub max_items: Option<u32>,
    pub unique: bool,
    pub min: Option<i64>,
    pub max: Option<RangeUpperBound>,
    pub pattern: Option<String>,
    pub url: bool,
    pub email: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum RangeUpperBound {
    Included(i64),
    Excluded(i64),
}

impl FieldAttrs {
    pub(crate) fn from_attrs(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut out = Self::default();
        let mut seen = HashSet::new();
        for attr in attrs.iter().filter(|a| a.path().is_ident("field")) {
            let entries: Punctuated<FieldEntry, Token![,]> =
                attr.parse_args_with(Punctuated::parse_terminated)?;
            for entry in entries {
                let name = match &entry {
                    FieldEntry::KeyValue { name, .. } | FieldEntry::Flag(name) => name,
                };
                record_setting(&mut seen, &name.to_string(), name.span())?;
                entry.apply(&mut out)?;
            }
        }
        let property = PropertyAttrs::from_attrs(attrs)?;
        property.apply_to(&mut out)?;
        Ok(out)
    }
}

impl ValidateAttrs {
    pub(crate) fn from_attrs(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut out = Self::default();
        let mut seen = HashSet::new();
        for attr in attrs.iter().filter(|a| a.path().is_ident("validate")) {
            let entries: Punctuated<SpannedEntry<ValidateEntry>, Token![,]> =
                attr.parse_args_with(Punctuated::parse_terminated)?;
            for entry in entries {
                record_setting(&mut seen, &entry.value.setting(), entry.span)?;
                entry.value.apply(&mut out)?;
            }
        }
        let property = PropertyAttrs::from_attrs(attrs)?;
        property.apply_validation_to(&mut out)?;
        Ok(out)
    }
}

struct SpannedEntry<T> {
    value: T,
    span: Span,
}

impl<T: Parse> Parse for SpannedEntry<T> {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let span = input.span();
        Ok(Self {
            value: input.parse()?,
            span,
        })
    }
}

fn record_setting(seen: &mut HashSet<String>, name: &str, span: Span) -> syn::Result<()> {
    if !seen.insert(name.to_owned()) {
        return Err(syn::Error::new(
            span,
            format!("duplicate property setting `{name}`"),
        ));
    }
    Ok(())
}

fn merge_flag(slot: &mut bool, value: bool, name: &str) -> syn::Result<()> {
    if value {
        if *slot {
            return Err(syn::Error::new(
                Span::call_site(),
                format!("duplicate property setting `{name}`"),
            ));
        }
        *slot = true;
    }
    Ok(())
}

/// Reject field-only helpers before an unsupported location can discard them.
pub(crate) fn reject_field_attributes(attrs: &[Attribute], location: &str) -> syn::Result<()> {
    if let Some(attr) = attrs.iter().find(|attr| {
        ["property", "field", "validate"]
            .iter()
            .any(|name| attr.path().is_ident(name))
    }) {
        return Err(syn::Error::new_spanned(
            attr,
            format!("property attributes are not supported on {location}"),
        ));
    }
    Ok(())
}

/// A skipped declaration cannot retain semantic or presentation intent.
pub(crate) fn check_skipped_attributes(attrs: &[Attribute]) -> syn::Result<()> {
    for attr in attrs {
        if attr.path().is_ident("property") || attr.path().is_ident("validate") {
            return Err(syn::Error::new_spanned(
                attr,
                "property attributes cannot be applied to a skipped declaration",
            ));
        }
        if attr.path().is_ident("field") {
            let entries =
                attr.parse_args_with(Punctuated::<FieldEntry, Token![,]>::parse_terminated)?;
            for entry in entries {
                if !matches!(entry, FieldEntry::Flag(ref name) if name == "skip") {
                    return Err(syn::Error::new_spanned(
                        attr,
                        "field attributes cannot be applied to a skipped declaration",
                    ));
                }
            }
        }
    }
    Ok(())
}

impl FieldAttrs {
    pub(crate) fn from_variant_attrs(
        attrs: &[Attribute],
        allow_description: bool,
    ) -> syn::Result<Self> {
        let property = PropertyAttrs::from_attrs(attrs)?;
        let field = Self::from_attrs(attrs)?;
        if property.input.is_some()
            || property.validate.is_some()
            || attrs.iter().any(|attr| attr.path().is_ident("validate"))
            || (!allow_description && field.description.is_some())
            || field.placeholder.is_some()
            || field.default.is_some()
            || field.hint.is_some()
            || field.secret
            || field.multiline
            || field.hidden
            || field.widget.is_some()
            || field.no_expression
            || field.expression_required
            || field.expressions.is_some()
            || field.enum_select
            || field.group.is_some()
            || field.skip
            || field.emit_as.is_some()
        {
            reject_field_attributes(
                attrs,
                "this enum variant (only supported display labels and descriptions may be used)",
            )?;
        }
        Ok(field)
    }
}

// ── Phase-5 #[property(...)] ────────────────────────────────────────────────

#[derive(Default, Debug)]
struct PropertyAttrs {
    display: Option<PropertyDisplay>,
    input: Option<PropertyInput>,
    validate: Option<PropertyValidate>,
    options: Option<Span>,
}

#[derive(Default, Debug)]
struct PropertyDisplay {
    label: Option<String>,
    description: Option<String>,
    placeholder: Option<String>,
    hint: Option<String>,
    group: Option<String>,
    widget: Option<PropertyWidget>,
    hidden: bool,
}

#[derive(Default, Debug)]
struct PropertyInput {
    required: bool,
    secret: bool,
    expressions: Option<PropertyExpressionMode>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PropertyExpressionMode {
    Allowed,
    Forbidden,
    Required,
}

#[derive(Default, Debug)]
struct PropertyValidate {
    non_empty: bool,
    min_length: Option<usize>,
    max_length: Option<usize>,
    min_items: Option<u32>,
    max_items: Option<u32>,
    unique: bool,
    min: Option<i64>,
    max: Option<RangeUpperBound>,
    pattern: Option<String>,
    url: bool,
    email: bool,
}

impl PropertyAttrs {
    fn from_attrs(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut out = Self::default();
        for attr in attrs.iter().filter(|a| a.path().is_ident("property")) {
            let entries: Punctuated<PropertySection, Token![,]> =
                attr.parse_args_with(Punctuated::parse_terminated)?;
            for entry in entries {
                entry.apply(&mut out)?;
            }
        }
        Ok(out)
    }

    fn apply_to(self, out: &mut FieldAttrs) -> syn::Result<()> {
        if let Some(display) = self.display {
            merge_opt(&mut out.label, display.label, "display(label)")?;
            merge_opt(
                &mut out.description,
                display.description,
                "display(description)",
            )?;
            merge_opt(
                &mut out.placeholder,
                display.placeholder,
                "display(placeholder)",
            )?;
            merge_opt(&mut out.hint, display.hint, "display(hint)")?;
            merge_opt(&mut out.group, display.group, "display(group)")?;
            if display.hidden {
                out.hidden = true;
            }
            if display.widget.is_some() && out.multiline {
                return Err(syn::Error::new(
                    Span::call_site(),
                    "`display(widget)` conflicts with `field(multiline)`",
                ));
            }
            merge_opt(&mut out.widget, display.widget, "display(widget)")?;
        }
        if let Some(input) = self.input {
            merge_flag(&mut out.secret, input.secret, "input(secret)")?;
            if input.expressions.is_some() && (out.no_expression || out.expression_required) {
                return Err(syn::Error::new(
                    Span::call_site(),
                    "`input(expressions)` conflicts with a legacy expression-mode setting",
                ));
            }
            out.expressions = input.expressions;
        }
        if let Some(span) = self.options {
            return Err(syn::Error::new(
                span,
                "`#[property(options(...))]` requires the checked loader/option-provider \
                 contract and is not implemented by this derive yet",
            ));
        }
        Ok(())
    }

    fn apply_validation_to(self, out: &mut ValidateAttrs) -> syn::Result<()> {
        if let Some(input) = self.input {
            merge_flag(&mut out.required, input.required, "input(required)")?;
        }
        if let Some(validate) = self.validate {
            if (validate.min_length.is_some() || validate.max_length.is_some())
                && (out.min_length.is_some() || out.max_length.is_some())
            {
                return Err(syn::Error::new(
                    Span::call_site(),
                    "duplicate property setting `length`",
                ));
            }
            if (validate.min.is_some() || validate.max.is_some())
                && (out.min.is_some() || out.max.is_some())
            {
                return Err(syn::Error::new(
                    Span::call_site(),
                    "duplicate property setting `range`",
                ));
            }
            merge_opt(
                &mut out.min_length,
                validate.min_length,
                "validate(length.min)",
            )?;
            merge_opt(
                &mut out.max_length,
                validate.max_length,
                "validate(length.max)",
            )?;
            merge_opt(&mut out.min, validate.min, "validate(range.min)")?;
            merge_opt(&mut out.max, validate.max, "validate(range.max)")?;
            merge_opt(&mut out.pattern, validate.pattern, "validate(pattern)")?;
            merge_flag(&mut out.url, validate.url, "validate(url)")?;
            merge_flag(&mut out.email, validate.email, "validate(email)")?;
            merge_opt(
                &mut out.min_items,
                validate.min_items,
                "validate(items.min)",
            )?;
            merge_opt(
                &mut out.max_items,
                validate.max_items,
                "validate(items.max)",
            )?;
            merge_flag(&mut out.unique, validate.unique, "validate(unique)")?;
            if validate.non_empty {
                out.min_length = Some(out.min_length.unwrap_or(0).max(1));
            }
        }
        if let (Some(min), Some(max)) = (out.min_length, out.max_length)
            && min > max
        {
            return Err(syn::Error::new(
                Span::call_site(),
                "length minimum exceeds maximum after combining non_empty and length bounds",
            ));
        }
        Ok(())
    }
}

fn merge_opt<T>(slot: &mut Option<T>, value: Option<T>, name: &str) -> syn::Result<()> {
    if let Some(value) = value {
        if slot.is_some() {
            return Err(syn::Error::new(
                Span::call_site(),
                format!("duplicate property setting `{name}`"),
            ));
        }
        *slot = Some(value);
    }
    Ok(())
}

enum PropertySection {
    Display(PropertyDisplay),
    Input(PropertyInput),
    Validate(PropertyValidate),
    Options(Span),
}

impl PropertySection {
    fn apply(self, out: &mut PropertyAttrs) -> syn::Result<()> {
        match self {
            Self::Display(section) => set_section(&mut out.display, section, "display"),
            Self::Input(section) => set_section(&mut out.input, section, "input"),
            Self::Validate(section) => set_section(&mut out.validate, section, "validate"),
            Self::Options(span) => {
                if out.options.replace(span).is_some() {
                    return Err(syn::Error::new(
                        span,
                        "duplicate `options(...)` section in `#[property]`",
                    ));
                }
                Ok(())
            },
        }
    }
}

fn set_section<T>(slot: &mut Option<T>, value: T, name: &str) -> syn::Result<()> {
    if slot.replace(value).is_some() {
        return Err(syn::Error::new(
            Span::call_site(),
            format!("duplicate `{name}(...)` section in `#[property]`"),
        ));
    }
    Ok(())
}

impl Parse for PropertySection {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: syn::Ident = input.parse()?;
        let content;
        syn::parenthesized!(content in input);
        match name.to_string().as_str() {
            "display" => Ok(Self::Display(content.parse()?)),
            "input" => Ok(Self::Input(content.parse()?)),
            "validate" => Ok(Self::Validate(content.parse()?)),
            "options" => {
                let _ = content.parse::<proc_macro2::TokenStream>()?;
                Ok(Self::Options(name.span()))
            },
            other => Err(syn::Error::new(
                name.span(),
                format!(
                    "unknown #[property(..)] section `{other}`; expected display(...), input(...), validate(...), or options(...)"
                ),
            )),
        }
    }
}

enum DisplayEntry {
    KeyValue { name: syn::Ident, value: Expr },
    Flag(syn::Ident),
}

impl Parse for PropertyDisplay {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut out = Self::default();
        let mut seen = HashSet::new();
        let entries: Punctuated<DisplayEntry, Token![,]> = Punctuated::parse_terminated(input)?;
        for entry in entries {
            let name = match &entry {
                DisplayEntry::KeyValue { name, .. } | DisplayEntry::Flag(name) => name,
            };
            record_setting(&mut seen, &name.to_string(), name.span())?;
            match entry {
                DisplayEntry::KeyValue { name, value } => {
                    let key = name.to_string();
                    match key.as_str() {
                        "label" => {
                            out.label = Some(expr_string_lit(&value, &name, "display(label)")?);
                        },
                        "description" => {
                            out.description =
                                Some(expr_string_lit(&value, &name, "display(description)")?);
                        },
                        "placeholder" => {
                            out.placeholder =
                                Some(expr_string_lit(&value, &name, "display(placeholder)")?);
                        },
                        "hint" => out.hint = Some(expr_string_lit(&value, &name, "display(hint)")?),
                        "group" => {
                            out.group = Some(expr_string_lit(&value, &name, "display(group)")?);
                        },
                        "widget" => out.widget = Some(parse_widget(&value, name.span())?),
                        "example" => {
                            return Err(syn::Error::new(
                                name.span(),
                                "`display(example = ...)` requires schema-owned presentation \
                                 example storage and is not implemented by this derive yet",
                            ));
                        },
                        other => {
                            return Err(syn::Error::new(
                                name.span(),
                                format!("unknown #[property(display(..))] key `{other}`"),
                            ));
                        },
                    }
                },
                DisplayEntry::Flag(name) => match name.to_string().as_str() {
                    "hidden" => out.hidden = true,
                    "visible_when" => {
                        return Err(syn::Error::new(
                            name.span(),
                            "`display(visible_when(...))` requires checked Condition support and is not implemented by this derive yet",
                        ));
                    },
                    other => {
                        return Err(syn::Error::new(
                            name.span(),
                            format!("unknown #[property(display(..))] flag `{other}`"),
                        ));
                    },
                },
            }
        }
        Ok(out)
    }
}

impl Parse for DisplayEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: syn::Ident = input.parse()?;
        if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            Ok(Self::KeyValue {
                name,
                value: input.parse()?,
            })
        } else if input.peek(syn::token::Paren) {
            if name != "visible_when" {
                return Err(syn::Error::new(
                    name.span(),
                    format!("display flag `{name}` does not accept arguments"),
                ));
            }
            let content;
            syn::parenthesized!(content in input);
            let _ = content.parse::<proc_macro2::TokenStream>()?;
            Ok(Self::Flag(name))
        } else {
            Ok(Self::Flag(name))
        }
    }
}

enum InputEntry {
    KeyValue { name: syn::Ident, value: syn::Ident },
    Flag(syn::Ident),
}

impl Parse for PropertyInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut out = Self::default();
        let mut seen = HashSet::new();
        let entries: Punctuated<InputEntry, Token![,]> = Punctuated::parse_terminated(input)?;
        for entry in entries {
            let name = match &entry {
                InputEntry::KeyValue { name, .. } | InputEntry::Flag(name) => name,
            };
            record_setting(&mut seen, &name.to_string(), name.span())?;
            match entry {
                InputEntry::Flag(name) => match name.to_string().as_str() {
                    "required" => out.required = true,
                    "secret" => out.secret = true,
                    "required_when" => {
                        return Err(syn::Error::new(
                            name.span(),
                            "`input(required_when(...))` requires checked Condition support and is not implemented by this derive yet",
                        ));
                    },
                    other => {
                        return Err(syn::Error::new(
                            name.span(),
                            format!("unknown #[property(input(..))] flag `{other}`"),
                        ));
                    },
                },
                InputEntry::KeyValue { name, value } => {
                    if !name.to_string().eq("expressions") {
                        return Err(syn::Error::new(
                            name.span(),
                            format!("unknown #[property(input(..))] key `{name}`"),
                        ));
                    }
                    out.expressions = Some(match value.to_string().as_str() {
                        "allowed" => PropertyExpressionMode::Allowed,
                        "forbidden" => PropertyExpressionMode::Forbidden,
                        "required" => PropertyExpressionMode::Required,
                        other => {
                            return Err(syn::Error::new(
                                value.span(),
                                format!(
                                    "unknown expression mode `{other}`; expected allowed, forbidden, or required"
                                ),
                            ));
                        },
                    });
                },
            }
        }
        Ok(out)
    }
}

impl Parse for InputEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: syn::Ident = input.parse()?;
        if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            Ok(Self::KeyValue {
                name,
                value: input.parse()?,
            })
        } else if input.peek(syn::token::Paren) {
            if name != "required_when" {
                return Err(syn::Error::new(
                    name.span(),
                    format!("input flag `{name}` does not accept arguments"),
                ));
            }
            let content;
            syn::parenthesized!(content in input);
            let _ = content.parse::<proc_macro2::TokenStream>()?;
            Ok(Self::Flag(name))
        } else {
            Ok(Self::Flag(name))
        }
    }
}

enum PropertyValidateEntry {
    Flag(syn::Ident),
    Length {
        min: Option<usize>,
        max: Option<usize>,
    },
    Items {
        min: Option<u32>,
        max: Option<u32>,
    },
    Range {
        min: Option<i64>,
        max: Option<RangeUpperBound>,
    },
    Pattern(String),
}

impl Parse for PropertyValidate {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut out = Self::default();
        let mut seen = HashSet::new();
        let entries: Punctuated<SpannedEntry<PropertyValidateEntry>, Token![,]> =
            Punctuated::parse_terminated(input)?;
        for entry in entries {
            let name = match &entry.value {
                PropertyValidateEntry::Flag(name) => name.to_string(),
                PropertyValidateEntry::Length { .. } => "length".to_owned(),
                PropertyValidateEntry::Items { .. } => "items".to_owned(),
                PropertyValidateEntry::Range { .. } => "range".to_owned(),
                PropertyValidateEntry::Pattern(_) => "pattern".to_owned(),
            };
            record_setting(&mut seen, &name, entry.span)?;
            match entry.value {
                PropertyValidateEntry::Flag(name) => match name.to_string().as_str() {
                    "non_empty" => out.non_empty = true,
                    "unique" => out.unique = true,
                    "url" => out.url = true,
                    "email" => out.email = true,
                    other => {
                        return Err(syn::Error::new(
                            name.span(),
                            format!("unknown #[property(validate(..))] flag `{other}`"),
                        ));
                    },
                },
                PropertyValidateEntry::Length { min, max } => {
                    out.min_length = min;
                    out.max_length = max;
                },
                PropertyValidateEntry::Items { min, max } => {
                    out.min_items = min;
                    out.max_items = max;
                },
                PropertyValidateEntry::Range { min, max } => {
                    out.min = min;
                    out.max = max;
                },
                PropertyValidateEntry::Pattern(pattern) => out.pattern = Some(pattern),
            }
        }
        Ok(out)
    }
}

impl Parse for PropertyValidateEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: syn::Ident = input.parse()?;
        let key = name.to_string();
        if input.peek(syn::token::Paren) {
            let content;
            syn::parenthesized!(content in input);
            match key.as_str() {
                "items" => parse_items(&content),
                "unique" => Err(syn::Error::new(
                    name.span(),
                    "validation flag `unique` does not accept arguments",
                )),
                "length" => match parse_length(&content)? {
                    ValidateEntry::Length { min, max } => Ok(Self::Length { min, max }),
                    _ => Err(syn::Error::new(
                        name.span(),
                        "internal property parser invariant failed for length",
                    )),
                },
                "range" => match parse_range(&content)? {
                    ValidateEntry::Range { min, max } => Ok(Self::Range { min, max }),
                    _ => Err(syn::Error::new(
                        name.span(),
                        "internal property parser invariant failed for range",
                    )),
                },
                other => Err(syn::Error::new(
                    name.span(),
                    format!("unknown #[property(validate(..))] function `{other}`"),
                )),
            }
        } else if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            let lit: Lit = input.parse()?;
            if key == "pattern" {
                Ok(Self::Pattern(string_lit(&lit, &name, "validate(pattern)")?))
            } else {
                Err(syn::Error::new(
                    name.span(),
                    format!("unknown #[property(validate(..))] option `{key}`"),
                ))
            }
        } else {
            Ok(Self::Flag(name))
        }
    }
}

fn parse_items(input: ParseStream) -> syn::Result<PropertyValidateEntry> {
    let span = input.span();
    let entries: Punctuated<(syn::Ident, u32), Token![,]> = input.parse_terminated(
        |entry| {
            let name: syn::Ident = entry.parse()?;
            if name != "min" && name != "max" {
                return Err(syn::Error::new(
                    name.span(),
                    "items key must be `min` or `max`",
                ));
            }
            entry.parse::<Token![=]>()?;
            if entry.peek(Token![-]) {
                return Err(entry.error("item counts require a non-negative integer literal"));
            }
            let literal: LitInt = entry
                .parse()
                .map_err(|_| entry.error("item counts require a non-negative integer literal"))?;
            let count = literal.base10_parse::<u32>().map_err(|_| {
                syn::Error::new(
                    literal.span(),
                    "item count must fit in u32 (0..=4294967295)",
                )
            })?;
            Ok((name, count))
        },
        Token![,],
    )?;
    let mut min = None;
    let mut max = None;
    let mut seen = HashSet::new();
    for (name, count) in entries {
        record_setting(&mut seen, &format!("items.{name}"), name.span())?;
        if name == "min" {
            min = Some(count);
        } else {
            max = Some(count);
        }
    }
    if min.is_none() && max.is_none() {
        return Err(syn::Error::new(span, "items requires at least one bound"));
    }
    if let (Some(minimum), Some(maximum)) = (min, max)
        && minimum > maximum
    {
        return Err(syn::Error::new(span, "items minimum exceeds maximum"));
    }
    Ok(PropertyValidateEntry::Items { min, max })
}

fn parse_widget(value: &Expr, span: Span) -> syn::Result<PropertyWidget> {
    let raw = match value {
        Expr::Path(path) if path.path.segments.len() == 1 => {
            path.path.segments[0].ident.to_string()
        },
        Expr::Lit(ExprLit {
            lit: Lit::Str(value),
            ..
        }) => value.value(),
        _ => {
            return Err(syn::Error::new(
                span,
                "`display(widget = ..)` expects a widget token such as `textarea`",
            ));
        },
    };
    Ok(match raw.as_str() {
        "auto" => PropertyWidget::Auto,
        "text" => PropertyWidget::Text,
        "textarea" => PropertyWidget::Textarea,
        "password" => PropertyWidget::Password,
        "number" => PropertyWidget::Number,
        "checkbox" => PropertyWidget::Checkbox,
        "select" => PropertyWidget::Select,
        "radio" => PropertyWidget::Radio,
        "object" => PropertyWidget::Object,
        "list" => PropertyWidget::List,
        other => {
            return Err(syn::Error::new(
                span,
                format!(
                    "unknown property widget `{other}`; expected auto, text, textarea, password, number, checkbox, select, radio, object, or list"
                ),
            ));
        },
    })
}

fn expr_string_lit(value: &Expr, name: &syn::Ident, field: &str) -> syn::Result<String> {
    if let Expr::Lit(ExprLit {
        lit: Lit::Str(value),
        ..
    }) = value
    {
        Ok(value.value())
    } else {
        Err(syn::Error::new(
            name.span(),
            format!("`{field}` expects a string literal"),
        ))
    }
}

fn string_lit(value: &Lit, name: &syn::Ident, field: &str) -> syn::Result<String> {
    if let Lit::Str(s) = value {
        Ok(s.value())
    } else {
        Err(syn::Error::new(
            name.span(),
            format!("`{field}` expects a string literal"),
        ))
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
                    "emit_as" => out.emit_as = Some(string_lit(&value, "emit_as")?),
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
        max: Option<RangeUpperBound>,
    },
    Pattern(String),
}

impl ValidateEntry {
    fn setting(&self) -> String {
        match self {
            Self::Flag(name) => name.to_string(),
            Self::Length { .. } => "length".to_owned(),
            Self::Range { .. } => "range".to_owned(),
            Self::Pattern(_) => "pattern".to_owned(),
        }
    }

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
    let mut seen = HashSet::new();
    let entries: Punctuated<LengthEntry, Token![,]> = Punctuated::parse_terminated(input)?;
    for entry in entries {
        match entry {
            LengthEntry::Min(v, span) => {
                record_setting(&mut seen, "length.min", span)?;
                min = Some(v);
            },
            LengthEntry::Max(v, span) => {
                record_setting(&mut seen, "length.max", span)?;
                max = Some(v);
            },
        }
    }
    if min.is_none() && max.is_none() {
        return Err(syn::Error::new(span, "length requires at least one bound"));
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
    Min(usize, Span),
    Max(usize, Span),
}

impl Parse for LengthEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: syn::Ident = input.parse()?;
        input.parse::<Token![=]>()?;
        let lit: LitInt = input.parse()?;
        let v: usize = lit.base10_parse()?;
        match name.to_string().as_str() {
            "min" => Ok(LengthEntry::Min(v, name.span())),
            "max" => Ok(LengthEntry::Max(v, name.span())),
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
                (Some(end), syn::RangeLimits::Closed(_)) => {
                    Some(RangeUpperBound::Included(lit_to_i64(end)?))
                },
                (Some(end), syn::RangeLimits::HalfOpen(_)) => {
                    Some(RangeUpperBound::Excluded(lit_to_i64(end)?))
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
    if min.is_none() && max.is_none() {
        return Err(syn::Error::new(span, "range requires at least one bound"));
    }
    if let (Some(min_v), Some(RangeUpperBound::Included(max_v))) = (min, max)
        && min_v > max_v
    {
        return Err(syn::Error::new(
            span,
            format!("#[validate(range(..))]: min ({min_v}) cannot exceed max ({max_v})"),
        ));
    }
    if let (Some(min_v), Some(RangeUpperBound::Excluded(max_v))) = (min, max)
        && min_v >= max_v
    {
        return Err(syn::Error::new(
            span,
            format!(
                "#[validate(range(..))]: min ({min_v}) must be less than exclusive max ({max_v})"
            ),
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
    /// Wire-level custom-rule expression strings.
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
            // `reserved` is a valid option used with the wrong (assignment) form —
            // point at the list syntax instead of claiming the option is unknown.
            "reserved" => Err(syn::Error::new(
                name.span(),
                "#[schema(reserved)] requires list syntax: write `reserved(\"key\")`, \
                 not `reserved = \"key\"`",
            )),
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

/// The subset of `#[serde(...)]` attributes the schema derive reads: the ones
/// that affect the schema key (`rename_all`, `rename`, `skip`, `flatten`) plus
/// `alias`, which does not change the key but is an alternative wire key serde
/// still deserializes into the field — so a reserved key must also reject it.
#[derive(Default)]
pub(crate) struct SerdeAttrs {
    pub rename_all: Option<RenameRule>,
    pub rename: Option<String>,
    pub skip: bool,
    /// `Some(span)` when `#[serde(flatten)]` is present — used to anchor the
    /// "flatten not yet supported" compile error at the attribute.
    pub flatten_span: Option<Span>,
    /// `#[serde(alias = "..")]` keys (deserialize-only alternative wire keys).
    /// Empty unless the field carries one or more aliases.
    pub aliases: Vec<String>,
    /// `#[serde(tag = "..")]` — the discriminant key. On an enum container this
    /// selects internally- (no `content`) or adjacently- (`with content`) tagged
    /// representation; the union derive reads it to record `SerdeTagging`.
    pub tag: Option<String>,
    /// `#[serde(content = "..")]` — the payload key for adjacent tagging.
    pub content: Option<String>,
    /// `#[serde(untagged)]` — present on the enum container.
    pub untagged: bool,
    /// `Some(span)` when `#[serde(rename_all_fields = ..)]` is present on an enum
    /// container. The union derive does not yet honor it (it would rename every
    /// struct-variant field), so it is rejected rather than silently ignored —
    /// ignoring it would desync the schema key from the wire key (a C1 break).
    pub rename_all_fields_span: Option<Span>,
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
                    Meta::NameValue(nv) if nv.path.is_ident("alias") => {
                        out.aliases.push(expr_str(&nv.value, "alias")?);
                    },
                    Meta::NameValue(nv) if nv.path.is_ident("tag") => {
                        out.tag = Some(expr_str(&nv.value, "tag")?);
                    },
                    Meta::NameValue(nv) if nv.path.is_ident("content") => {
                        out.content = Some(expr_str(&nv.value, "content")?);
                    },
                    Meta::Path(p) if p.is_ident("untagged") => {
                        out.untagged = true;
                    },
                    Meta::NameValue(nv) if nv.path.is_ident("rename_all_fields") => {
                        out.rename_all_fields_span = Some(nv.span());
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
