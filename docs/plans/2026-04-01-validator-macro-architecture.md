# Validator Macro Architecture Refactor

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Restructure the `nebula-validator-macros` derive macro from a monolithic 1335-line function into a clean 3-phase pipeline (parse → check → emit), following the garde architecture pattern.

**Architecture:** Split `validator.rs` into 4 modules: `model.rs` (IR types), `parse.rs` (attributes → IR), `check.rs` (semantic validation), `emit.rs` (IR → TokenStream). Each validation rule becomes a variant in a `Rule` enum, Option-wrapping and message-override logic are centralized into single helper functions.

**Tech Stack:** `syn`, `quote`, `proc-macro2`, `nebula-macro-support` (attrs, diag)

---

## Current Problems

1. **Monolithic `expand()`** — 1050+ lines, parses attrs and generates code in one pass
2. **Message override duplication** — `before/after/last_mut` pattern copy-pasted ~20 times
3. **Option-wrapping duplication** — `if is_option { ... } else { ... }` in every validator
4. **`each(...)` re-implements field validators** — complete duplication of validation codegen
5. **No IR** — can't validate attribute combinations before codegen

## Target Architecture

```
lib.rs          — entry point, calls parse → check → emit
model.rs        — IR types: Rule enum, FieldRules, ContainerRules, EachRules
parse.rs        — DeriveInput → model::ValidatorInput (raw IR)
check.rs        — semantic validation of parsed IR (type checks, conflicts)
emit.rs         — model::ValidatorInput → TokenStream
```

## Invariants

- **Zero behavior change** — generated code must produce identical validation behavior
- **Same attribute syntax** — `#[validator(message = "...")]` and `#[validate(...)]` unchanged
- **Same trait impls generated** — `Validate<Self>`, `SelfValidating`, `validate_fields()`
- **Error spans preserved** — compile errors still point to the right attribute/field
- The `nebula-macro-support::validation_codegen` helpers can be deprecated incrementally; codegen moves into `emit.rs`

---

### Task 1: Create `model.rs` — the IR types

**Files:**
- Create: `crates/validator/macros/src/model.rs`

**Step 1: Write the IR types**

```rust
//! Intermediate representation for the Validator derive macro.

use proc_macro2::{Span, TokenStream as TokenStream2};
use syn::{Ident, Type};

/// Top-level parsed input for the Validator derive.
#[derive(Debug)]
pub struct ValidatorInput {
    /// The struct identifier.
    pub ident: Ident,
    /// Generic parameters from the struct.
    pub generics: syn::Generics,
    /// Container-level attributes from `#[validator(...)]`.
    pub container: ContainerAttrs,
    /// Per-field rules from `#[validate(...)]`.
    pub fields: Vec<FieldDef>,
}

/// Container-level attributes (`#[validator(...)]`).
#[derive(Debug)]
pub struct ContainerAttrs {
    /// Custom root-level error message.
    pub message: String,
}

impl Default for ContainerAttrs {
    fn default() -> Self {
        Self {
            message: "validation failed".to_string(),
        }
    }
}

/// A single named struct field with its validation rules.
#[derive(Debug)]
pub struct FieldDef {
    /// The field identifier.
    pub ident: Ident,
    /// The field's declared type (as written by the user).
    pub ty: Type,
    /// Whether the field type is `Option<T>`.
    pub is_option: bool,
    /// The inner type (unwrapped from Option if applicable).
    pub inner_ty: Type,
    /// Field-level custom message override (applies to all rules on this field).
    pub message: Option<String>,
    /// The validation rules for this field.
    pub rules: Vec<Rule>,
    /// Element-level rules for `each(...)` on Vec fields.
    pub each_rules: Option<EachRules>,
    /// Span for error reporting.
    pub span: Span,
}

/// Element-level validation for `each(...)` on Vec fields.
#[derive(Debug)]
pub struct EachRules {
    /// The element type inside `Vec<T>`.
    pub element_ty: Type,
    /// Validation rules to apply to each element.
    pub rules: Vec<Rule>,
}

/// A single validation rule parsed from attributes.
#[derive(Debug)]
pub enum Rule {
    /// `required` — field must be `Some` (only valid on Option fields).
    Required,
    /// `min_length = N`
    MinLength(usize),
    /// `max_length = N`
    MaxLength(usize),
    /// `exact_length = N`
    ExactLength(usize),
    /// `length_range(min = N, max = M)`
    LengthRange { min: usize, max: usize },
    /// `min = <lit>`
    Min(TokenStream2),
    /// `max = <lit>`
    Max(TokenStream2),
    /// `min_size = N` (Vec)
    MinSize(usize),
    /// `max_size = N` (Vec)
    MaxSize(usize),
    /// `exact_size = N` (Vec)
    ExactSize(usize),
    /// `size_range(min = N, max = M)` (Vec)
    SizeRange { min: usize, max: usize },
    /// `not_empty_collection` (Vec)
    NotEmptyCollection,
    /// Built-in string format flag (e.g., `email`, `url`, `not_empty`).
    StringFormat(StringFormat),
    /// `contains = "..."`, `starts_with = "..."`, `ends_with = "..."`
    StringFactory { kind: StringFactoryKind, arg: String },
    /// `is_true`
    IsTrue,
    /// `is_false`
    IsFalse,
    /// `regex = "pattern"`
    Regex(String),
    /// `nested` — delegates to SelfValidating::check()
    Nested,
    /// `custom = expr`
    Custom(TokenStream2),
}

/// Built-in string format validators (zero-arg factories).
#[derive(Debug, Clone, Copy)]
pub enum StringFormat {
    NotEmpty,
    Alphanumeric,
    Alphabetic,
    Numeric,
    Lowercase,
    Uppercase,
    Email,
    Url,
    Ipv4,
    Ipv6,
    IpAddr,
    Hostname,
    Uuid,
    Date,
    DateTime,
    Time,
}

/// String factory validators (one-arg factories).
#[derive(Debug, Clone, Copy)]
pub enum StringFactoryKind {
    Contains,
    StartsWith,
    EndsWith,
}
```

**Step 2: Register the module**

In `crates/validator/macros/src/lib.rs`, add `mod model;` (don't change anything else yet).

**Step 3: Run check to verify it compiles**

Run: `rtk cargo check -p nebula-validator-macros`
Expected: PASS

**Step 4: Commit**

```bash
rtk git add crates/validator/macros/src/model.rs crates/validator/macros/src/lib.rs
rtk git commit -m "refactor(validator-macros): add IR model types for 3-phase pipeline"
```

---

### Task 2: Create `parse.rs` — attribute parsing into IR

**Files:**
- Create: `crates/validator/macros/src/parse.rs`

**Step 1: Write the parsing module**

This module converts `DeriveInput` into `ValidatorInput`. All attribute parsing logic moves here from the old `validator.rs`. Key responsibilities:

- Parse `#[validator(message = "...")]` container attrs
- For each field, parse `#[validate(...)]` into `Vec<Rule>`
- Detect `Option<T>` and `Vec<T>` wrappers and set flags on `FieldDef`
- Parse `each(...)` sub-attrs into `EachRules`
- Parse `custom = expr` into `Rule::Custom`

```rust
//! Attribute parsing — converts DeriveInput into model::ValidatorInput.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{DeriveInput, Data, Type};

use nebula_macro_support::{attrs, diag};
use nebula_macro_support::validation_codegen::{
    is_option_type, parse_number_lit, parse_usize, value_token,
};

use crate::model::*;

/// Parse a `DeriveInput` into the macro's IR.
pub fn parse(input: &DeriveInput) -> syn::Result<ValidatorInput> {
    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            syn::Fields::Named(fields) => &fields.named,
            _ => {
                return Err(syn::Error::new(
                    input.ident.span(),
                    "Validator derive requires a struct with named fields",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new(
                input.ident.span(),
                "Validator derive can only be used on structs",
            ));
        }
    };

    let validator_attrs = attrs::parse_attrs(&input.attrs, "validator")?;
    let container = ContainerAttrs {
        message: validator_attrs
            .get_string("message")
            .unwrap_or_else(|| "validation failed".to_string()),
    };

    let mut field_defs = Vec::with_capacity(fields.len());
    for field in fields {
        let Some(ident) = &field.ident else { continue };
        let validate_attrs = attrs::parse_attrs(&field.attrs, "validate")?;

        let is_option = is_option_type(&field.ty);
        let inner_ty = option_inner_type(&field.ty)
            .cloned()
            .unwrap_or_else(|| field.ty.clone());
        let message = validate_attrs.get_string("message");

        let rules = parse_field_rules(&validate_attrs, &field.ty, &inner_ty)?;
        let each_rules = parse_each_rules(&validate_attrs, &field.ty, &inner_ty)?;

        field_defs.push(FieldDef {
            ident: ident.clone(),
            ty: field.ty.clone(),
            is_option,
            inner_ty,
            message,
            rules,
            each_rules,
            span: ident.span(),
        });
    }

    Ok(ValidatorInput {
        ident: input.ident.clone(),
        generics: input.generics.clone(),
        container,
        fields: field_defs,
    })
}

/// Parse all validation rules from a field's `#[validate(...)]` attrs.
fn parse_field_rules(
    attrs: &attrs::AttrArgs,
    original_ty: &Type,
    inner_ty: &Type,
) -> syn::Result<Vec<Rule>> {
    let is_option = is_option_type(original_ty);
    let is_string = is_string_type(inner_ty);
    let is_bool = is_bool_type(inner_ty);
    let is_vec = vec_inner_type(inner_ty).is_some();

    let mut rules = Vec::new();

    // required (only on Option)
    if attrs.has_flag("required") && is_option {
        rules.push(Rule::Required);
    }

    // length validators
    if let Some(v) = parse_usize(attrs, "min_length")? {
        rules.push(Rule::MinLength(v));
    }
    if let Some(v) = parse_usize(attrs, "max_length")? {
        rules.push(Rule::MaxLength(v));
    }
    if let Some(v) = parse_usize(attrs, "exact_length")? {
        rules.push(Rule::ExactLength(v));
    }
    if let Some((min, max)) = parse_min_max_list(attrs, "length_range")? {
        require_string_type(original_ty, is_string, "length_range(...)")?;
        rules.push(Rule::LengthRange { min, max });
    }

    // numeric comparisons
    if let Some(v) = parse_number_lit(attrs, "min")? {
        rules.push(Rule::Min(v));
    }
    if let Some(v) = parse_number_lit(attrs, "max")? {
        rules.push(Rule::Max(v));
    }

    // collection size validators
    if let Some(v) = parse_usize(attrs, "min_size")? {
        require_vec_type(original_ty, is_vec, "min_size")?;
        rules.push(Rule::MinSize(v));
    }
    if let Some(v) = parse_usize(attrs, "max_size")? {
        require_vec_type(original_ty, is_vec, "max_size")?;
        rules.push(Rule::MaxSize(v));
    }
    if let Some(v) = parse_usize(attrs, "exact_size")? {
        require_vec_type(original_ty, is_vec, "exact_size")?;
        rules.push(Rule::ExactSize(v));
    }
    if let Some((min, max)) = parse_min_max_list(attrs, "size_range")? {
        require_vec_type(original_ty, is_vec, "size_range(...)")?;
        rules.push(Rule::SizeRange { min, max });
    }
    if attrs.has_flag("not_empty_collection") {
        require_vec_type(original_ty, is_vec, "not_empty_collection")?;
        rules.push(Rule::NotEmptyCollection);
    }

    // string format flags
    for (flag, format) in string_format_flags() {
        if attrs.has_flag(flag) {
            require_string_type(original_ty, is_string, flag)?;
            rules.push(Rule::StringFormat(format));
        }
    }

    // string factory validators
    for (key, kind) in string_factory_keys() {
        if let Some(arg) = attrs.get_string(key) {
            require_string_type(original_ty, is_string, &format!("{key} = ..."))?;
            rules.push(Rule::StringFactory { kind, arg });
        }
    }

    // boolean validators
    if attrs.has_flag("is_true") {
        require_bool_type(original_ty, is_bool, "is_true")?;
        rules.push(Rule::IsTrue);
    }
    if attrs.has_flag("is_false") {
        require_bool_type(original_ty, is_bool, "is_false")?;
        rules.push(Rule::IsFalse);
    }

    // regex
    if let Some(pattern) = attrs.get_string("regex") {
        require_string_type(original_ty, is_string, "regex = ...")?;
        rules.push(Rule::Regex(pattern));
    }

    // nested
    if attrs.has_flag("nested") {
        rules.push(Rule::Nested);
    }

    // custom
    if let Some(expr) = parse_custom_validator_expr(attrs)? {
        rules.push(Rule::Custom(expr));
    }

    Ok(rules)
}

/// Parse `each(...)` sub-attributes into element-level rules.
fn parse_each_rules(
    attrs: &attrs::AttrArgs,
    original_ty: &Type,
    inner_ty: &Type,
) -> syn::Result<Option<EachRules>> {
    let Some(values) = attrs.get_list_values("each") else {
        return Ok(None);
    };

    // Resolve the Vec element type
    let source_ty = option_inner_type(original_ty).unwrap_or(original_ty);
    let element_ty = vec_inner_type(source_ty).ok_or_else(|| {
        syn::Error::new_spanned(
            original_ty,
            "`each(...)` is supported for `Vec<T>` and `Option<Vec<T>>` fields",
        )
    })?;

    // Convert list values to AttrArgs for reuse
    let each_attrs = parse_each_attr_args(values)?;
    let is_string = is_string_type(element_ty);
    let is_bool = is_bool_type(element_ty);

    let mut rules = Vec::new();

    // length validators on elements
    if let Some(v) = parse_usize(&each_attrs, "min_length")? {
        rules.push(Rule::MinLength(v));
    }
    if let Some(v) = parse_usize(&each_attrs, "max_length")? {
        rules.push(Rule::MaxLength(v));
    }
    if let Some(v) = parse_usize(&each_attrs, "exact_length")? {
        rules.push(Rule::ExactLength(v));
    }

    // numeric comparisons on elements
    if let Some(v) = parse_number_lit(&each_attrs, "min")? {
        rules.push(Rule::Min(v));
    }
    if let Some(v) = parse_number_lit(&each_attrs, "max")? {
        rules.push(Rule::Max(v));
    }

    // string format flags on elements
    for (flag, format) in string_format_flags() {
        if each_attrs.has_flag(flag) {
            if !is_string {
                return Err(syn::Error::new_spanned(
                    original_ty,
                    format!("`each({flag})` requires `Vec<String>` or `Option<Vec<String>>`"),
                ));
            }
            rules.push(Rule::StringFormat(format));
        }
    }

    // string factory validators on elements
    for (key, kind) in string_factory_keys() {
        if let Some(arg) = each_attrs.get_string(key) {
            if !is_string {
                return Err(syn::Error::new_spanned(
                    original_ty,
                    format!("`each({key} = ...)` requires `Vec<String>` or `Option<Vec<String>>`"),
                ));
            }
            rules.push(Rule::StringFactory { kind, arg });
        }
    }

    // regex on elements
    if let Some(pattern) = each_attrs.get_string("regex") {
        if !is_string {
            return Err(syn::Error::new_spanned(
                original_ty,
                "`each(regex = ...)` requires `Vec<String>` or `Option<Vec<String>>`",
            ));
        }
        rules.push(Rule::Regex(pattern));
    }

    // nested on elements
    if each_attrs.has_flag("nested") {
        rules.push(Rule::Nested);
    }

    // custom on elements
    if let Some(expr) = parse_custom_validator_expr(&each_attrs)? {
        rules.push(Rule::Custom(expr));
    }

    Ok(Some(EachRules {
        element_ty: element_ty.clone(),
        rules,
    }))
}

// ---------------------------------------------------------------------------
// Helpers (moved from old validator.rs)
// ---------------------------------------------------------------------------

fn option_inner_type(ty: &Type) -> Option<&Type> {
    let Type::Path(type_path) = ty else { return None };
    let segment = type_path.path.segments.last()?;
    if segment.ident != "Option" { return None }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else { return None };
    let syn::GenericArgument::Type(inner) = args.args.first()? else { return None };
    Some(inner)
}

fn vec_inner_type(ty: &Type) -> Option<&Type> {
    let Type::Path(type_path) = ty else { return None };
    let segment = type_path.path.segments.last()?;
    if segment.ident != "Vec" { return None }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else { return None };
    let syn::GenericArgument::Type(inner) = args.args.first()? else { return None };
    Some(inner)
}

fn is_string_type(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else { return false };
    type_path.path.segments.last()
        .is_some_and(|s| s.ident == "String")
}

fn is_bool_type(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else { return false };
    type_path.path.segments.last()
        .is_some_and(|s| s.ident == "bool")
}

fn require_string_type(ty: &Type, is_string: bool, attr_name: &str) -> syn::Result<()> {
    if !is_string {
        return Err(syn::Error::new_spanned(
            ty,
            format!("`{attr_name}` requires `String` or `Option<String>` fields"),
        ));
    }
    Ok(())
}

fn require_vec_type(ty: &Type, is_vec: bool, attr_name: &str) -> syn::Result<()> {
    if !is_vec {
        return Err(syn::Error::new_spanned(
            ty,
            format!("`{attr_name}` requires `Vec<T>` or `Option<Vec<T>>` fields"),
        ));
    }
    Ok(())
}

fn require_bool_type(ty: &Type, is_bool: bool, attr_name: &str) -> syn::Result<()> {
    if !is_bool {
        return Err(syn::Error::new_spanned(
            ty,
            format!("`{attr_name}` requires `bool` or `Option<bool>` fields"),
        ));
    }
    Ok(())
}

fn string_format_flags() -> Vec<(&'static str, StringFormat)> {
    vec![
        ("not_empty", StringFormat::NotEmpty),
        ("alphanumeric", StringFormat::Alphanumeric),
        ("alphabetic", StringFormat::Alphabetic),
        ("numeric", StringFormat::Numeric),
        ("lowercase", StringFormat::Lowercase),
        ("uppercase", StringFormat::Uppercase),
        ("email", StringFormat::Email),
        ("url", StringFormat::Url),
        ("ipv4", StringFormat::Ipv4),
        ("ipv6", StringFormat::Ipv6),
        ("ip_addr", StringFormat::IpAddr),
        ("hostname", StringFormat::Hostname),
        ("uuid", StringFormat::Uuid),
        ("date", StringFormat::Date),
        ("date_time", StringFormat::DateTime),
        ("time", StringFormat::Time),
    ]
}

fn string_factory_keys() -> Vec<(&'static str, StringFactoryKind)> {
    vec![
        ("contains", StringFactoryKind::Contains),
        ("starts_with", StringFactoryKind::StartsWith),
        ("ends_with", StringFactoryKind::EndsWith),
    ]
}

fn parse_custom_validator_expr(args: &attrs::AttrArgs) -> syn::Result<Option<TokenStream2>> {
    let Some(value) = args.get_value("custom") else {
        return Ok(None);
    };

    let expr = match value {
        attrs::AttrValue::Ident(ident) => quote!(#ident),
        attrs::AttrValue::Tokens(tokens) => tokens.clone(),
        attrs::AttrValue::Lit(syn::Lit::Str(s)) => {
            let parsed = syn::parse_str::<syn::Expr>(&s.value())
                .map_err(|e| diag::error_spanned(s, format!("invalid custom validator: {e}")))?;
            quote!(#parsed)
        }
        _ => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`custom` must be a function path or string path",
            ));
        }
    };

    Ok(Some(expr))
}

fn parse_min_max_list(
    attrs: &attrs::AttrArgs,
    key: &str,
) -> syn::Result<Option<(usize, usize)>> {
    let Some(values) = attrs.get_list_values(key) else {
        return Ok(None);
    };

    let mut min: Option<usize> = None;
    let mut max: Option<usize> = None;

    for value in values {
        let item = parse_list_item_to_attr_item(value)?;
        let attrs::AttrItem::KeyValue { key: entry_key, value } = item else {
            return Err(diag::error_spanned(
                &value_token(value),
                format!("`{key}` expects key-value entries like `{key}(min = 1, max = 10)`"),
            ));
        };

        let parsed = match value {
            attrs::AttrValue::Lit(syn::Lit::Int(int)) => {
                int.base10_parse::<usize>().map_err(|_| {
                    diag::error_spanned(&int, format!("`{key}` bounds must be positive integers"))
                })?
            }
            other => {
                return Err(diag::error_spanned(
                    &value_token(&other),
                    format!("`{key}` bounds must be integer literals"),
                ));
            }
        };

        if entry_key == "min" {
            min = Some(parsed);
        } else if entry_key == "max" {
            max = Some(parsed);
        } else {
            return Err(syn::Error::new_spanned(
                entry_key,
                format!("`{key}` only supports `min` and `max` keys"),
            ));
        }
    }

    let min = min.ok_or_else(|| {
        syn::Error::new(proc_macro2::Span::call_site(), format!("`{key}` requires both `min` and `max`"))
    })?;
    let max = max.ok_or_else(|| {
        syn::Error::new(proc_macro2::Span::call_site(), format!("`{key}` requires both `min` and `max`"))
    })?;

    if min > max {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("`{key}` requires `min <= max`"),
        ));
    }

    Ok(Some((min, max)))
}

fn parse_each_attr_args(values: &[attrs::AttrValue]) -> syn::Result<attrs::AttrArgs> {
    let mut items = Vec::with_capacity(values.len());
    for value in values {
        items.push(parse_list_item_to_attr_item(value)?);
    }
    Ok(attrs::AttrArgs { items })
}

fn parse_list_item_to_attr_item(item: &attrs::AttrValue) -> syn::Result<attrs::AttrItem> {
    match item {
        attrs::AttrValue::Ident(ident) => Ok(attrs::AttrItem::Flag(ident.clone())),
        attrs::AttrValue::Lit(syn::Lit::Str(s)) => {
            let parsed = syn::parse_str::<syn::Expr>(&s.value())
                .map_err(|e| diag::error_spanned(s, format!("invalid each(...) entry: {e}")))?;
            parse_expr_to_attr_item(parsed, item)
        }
        attrs::AttrValue::Tokens(tokens) => {
            let parsed = syn::parse2::<syn::Expr>(tokens.clone())
                .map_err(|e| diag::error_spanned(tokens, format!("invalid each(...) entry: {e}")))?;
            parse_expr_to_attr_item(parsed, item)
        }
        attrs::AttrValue::Lit(other) => Err(diag::error_spanned(
            other,
            "unsupported each(...) entry; use flags or key-value entries",
        )),
    }
}

fn parse_expr_to_attr_item(
    expr: syn::Expr,
    span_source: &attrs::AttrValue,
) -> syn::Result<attrs::AttrItem> {
    match expr {
        syn::Expr::Path(path) => {
            if path.path.segments.len() == 1 && path.path.leading_colon.is_none() {
                Ok(attrs::AttrItem::Flag(
                    path.path.segments.into_iter().next().expect("segment").ident,
                ))
            } else {
                Err(diag::error_spanned(
                    &value_token(span_source),
                    "each(...) flags must be single identifiers",
                ))
            }
        }
        syn::Expr::Assign(assign) => {
            let syn::Expr::Path(left_path) = *assign.left else {
                return Err(diag::error_spanned(
                    &value_token(span_source),
                    "each(...) key-value entry must use identifier keys",
                ));
            };
            if left_path.path.segments.len() != 1 || left_path.path.leading_colon.is_some() {
                return Err(diag::error_spanned(
                    &value_token(span_source),
                    "each(...) key-value entry must use identifier keys",
                ));
            }
            let key = left_path.path.segments.into_iter().next().expect("segment").ident;
            let value = expr_to_attr_value(*assign.right);
            Ok(attrs::AttrItem::KeyValue { key, value })
        }
        _ => Err(diag::error_spanned(
            &value_token(span_source),
            "unsupported each(...) entry; use flags or key-value entries",
        )),
    }
}

fn expr_to_attr_value(expr: syn::Expr) -> attrs::AttrValue {
    match expr {
        syn::Expr::Path(path) => {
            if path.path.segments.len() == 1 && path.path.leading_colon.is_none() {
                attrs::AttrValue::Ident(
                    path.path.segments.into_iter().next().expect("segment").ident,
                )
            } else {
                attrs::AttrValue::Tokens(quote!(#path))
            }
        }
        syn::Expr::Lit(lit) => attrs::AttrValue::Lit(lit.lit),
        other => attrs::AttrValue::Tokens(quote!(#other)),
    }
}
```

**Step 2: Register the module**

In `lib.rs`, add `mod parse;`.

**Step 3: Run check**

Run: `rtk cargo check -p nebula-validator-macros`
Expected: PASS

**Step 4: Commit**

```bash
rtk git add crates/validator/macros/src/parse.rs crates/validator/macros/src/lib.rs
rtk git commit -m "refactor(validator-macros): add parse module — attributes to IR"
```

---

### Task 3: Create `emit.rs` — codegen from IR

**Files:**
- Create: `crates/validator/macros/src/emit.rs`

**Step 1: Write the emit module**

This is the core win — Option-wrapping and message-override are each handled in ONE place.

```rust
//! Code generation — converts ValidatorInput IR into TokenStream.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use crate::model::*;

/// Generate the full TokenStream from validated IR.
pub fn emit(input: &ValidatorInput) -> TokenStream2 {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let root_message = &input.container.message;

    let checks: Vec<TokenStream2> = input.fields.iter().flat_map(|field| {
        let mut tokens = Vec::new();

        // Emit each() loop BEFORE field-level rules (matches old ordering)
        if let Some(each) = &field.each_rules {
            tokens.push(emit_each_loop(field, each));
        }

        // Emit field-level rules
        for rule in &field.rules {
            tokens.push(emit_field_rule(field, rule));
        }

        tokens
    }).collect();

    quote! {
        impl #impl_generics #struct_name #ty_generics #where_clause {
            /// Validates this value using field-level `#[validate(...)]` rules.
            pub fn validate_fields(
                &self,
            ) -> ::std::result::Result<(), ::nebula_validator::foundation::ValidationErrors> {
                let input = self;
                let mut errors = ::nebula_validator::foundation::ValidationErrors::new();
                #(#checks)*

                if errors.has_errors() {
                    Err(errors)
                } else {
                    Ok(())
                }
            }
        }

        impl #impl_generics ::nebula_validator::foundation::Validate<#struct_name #ty_generics> for #struct_name #ty_generics #where_clause {
            fn validate(
                &self,
                input: &#struct_name #ty_generics,
            ) -> ::std::result::Result<(), ::nebula_validator::foundation::ValidationError> {
                let _ = self;
                input
                    .validate_fields()
                    .map_err(|errors| errors.into_single_error(#root_message))
            }
        }

        impl #impl_generics ::nebula_validator::combinators::SelfValidating for #struct_name #ty_generics #where_clause {
            fn check(&self) -> ::std::result::Result<(), ::nebula_validator::foundation::ValidationError> {
                self.validate(self)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Centralized Option-wrapping
// ---------------------------------------------------------------------------

/// Wraps a check in `if let Some(value) = ... { check }` when the field is Option.
/// The `value_expr` is the expression to use for the unwrapped value.
/// The `check` is the code that runs on the unwrapped value.
fn wrap_option(field: &FieldDef, check: TokenStream2) -> TokenStream2 {
    let field_name = &field.ident;
    if field.is_option {
        quote! {
            if let Some(value) = input.#field_name.as_ref() {
                #check
            }
        }
    } else {
        check
    }
}

// ---------------------------------------------------------------------------
// Centralized message override
// ---------------------------------------------------------------------------

/// Wraps a check with the field-level message override if present.
fn wrap_message(field: &FieldDef, check: TokenStream2) -> TokenStream2 {
    match &field.message {
        Some(message) => quote! {
            let before = errors.len();
            #check
            let after = errors.len();
            if after > before {
                if let Some(last) = errors.last_mut() {
                    last.message = ::std::borrow::Cow::Owned(#message.to_string());
                }
            }
        },
        None => check,
    }
}

// ---------------------------------------------------------------------------
// Field-level rule emission
// ---------------------------------------------------------------------------

/// Emit a single field-level rule.
fn emit_field_rule(field: &FieldDef, rule: &Rule) -> TokenStream2 {
    let field_name = &field.ident;
    let field_key = field_name.to_string();

    match rule {
        Rule::Required => {
            // Required is special — it checks the Option itself, no unwrapping
            let check = quote! {
                if input.#field_name.is_none() {
                    errors.add(::nebula_validator::foundation::ValidationError::required(#field_key));
                }
            };
            wrap_message(field, check)
        }

        Rule::MinLength(n) => {
            let inner = if field.is_option {
                quote! {
                    if value.len() < #n {
                        errors.add(::nebula_validator::foundation::ValidationError::min_length(#field_key, #n, value.len()));
                    }
                }
            } else {
                quote! {
                    let value = &input.#field_name;
                    if value.len() < #n {
                        errors.add(::nebula_validator::foundation::ValidationError::min_length(#field_key, #n, value.len()));
                    }
                }
            };
            wrap_message(field, wrap_option(field, inner))
        }

        Rule::MaxLength(n) => {
            let inner = if field.is_option {
                quote! {
                    if value.len() > #n {
                        errors.add(::nebula_validator::foundation::ValidationError::max_length(#field_key, #n, value.len()));
                    }
                }
            } else {
                quote! {
                    let value = &input.#field_name;
                    if value.len() > #n {
                        errors.add(::nebula_validator::foundation::ValidationError::max_length(#field_key, #n, value.len()));
                    }
                }
            };
            wrap_message(field, wrap_option(field, inner))
        }

        Rule::ExactLength(n) => {
            let inner = if field.is_option {
                quote! {
                    if value.len() != #n {
                        errors.add(::nebula_validator::foundation::ValidationError::exact_length(#field_key, #n, value.len()));
                    }
                }
            } else {
                quote! {
                    let value = &input.#field_name;
                    if value.len() != #n {
                        errors.add(::nebula_validator::foundation::ValidationError::exact_length(#field_key, #n, value.len()));
                    }
                }
            };
            wrap_message(field, wrap_option(field, inner))
        }

        Rule::LengthRange { min, max } => {
            let inner = if field.is_option {
                quote! {
                    match ::nebula_validator::validators::length_range(#min, #max) {
                        Ok(v) => {
                            if let Err(e) = ::nebula_validator::foundation::Validate::validate(&v, value.as_str()) {
                                errors.add(e.with_field(#field_key));
                            }
                        }
                        Err(e) => {
                            errors.add(e.with_field(#field_key));
                        }
                    }
                }
            } else {
                quote! {
                    match ::nebula_validator::validators::length_range(#min, #max) {
                        Ok(v) => {
                            if let Err(e) = ::nebula_validator::foundation::Validate::validate(&v, input.#field_name.as_str()) {
                                errors.add(e.with_field(#field_key));
                            }
                        }
                        Err(e) => {
                            errors.add(e.with_field(#field_key));
                        }
                    }
                }
            };
            wrap_message(field, wrap_option(field, inner))
        }

        Rule::Min(bound) => {
            let inner = if field.is_option {
                quote! {
                    if value < &#bound {
                        errors.add(
                            ::nebula_validator::foundation::ValidationError::new(
                                "min", format!("{} must be >= {}", #field_key, #bound),
                            ).with_field(#field_key)
                        );
                    }
                }
            } else {
                quote! {
                    let value = &input.#field_name;
                    if value < &#bound {
                        errors.add(
                            ::nebula_validator::foundation::ValidationError::new(
                                "min", format!("{} must be >= {}", #field_key, #bound),
                            ).with_field(#field_key)
                        );
                    }
                }
            };
            wrap_message(field, wrap_option(field, inner))
        }

        Rule::Max(bound) => {
            let inner = if field.is_option {
                quote! {
                    if value > &#bound {
                        errors.add(
                            ::nebula_validator::foundation::ValidationError::new(
                                "max", format!("{} must be <= {}", #field_key, #bound),
                            ).with_field(#field_key)
                        );
                    }
                }
            } else {
                quote! {
                    let value = &input.#field_name;
                    if value > &#bound {
                        errors.add(
                            ::nebula_validator::foundation::ValidationError::new(
                                "max", format!("{} must be <= {}", #field_key, #bound),
                            ).with_field(#field_key)
                        );
                    }
                }
            };
            wrap_message(field, wrap_option(field, inner))
        }

        Rule::MinSize(n) => {
            let element_type = vec_inner_type_from_field(field);
            let inner = emit_vec_validator(field, quote! {
                ::nebula_validator::validators::min_size::<#element_type>(#n)
            }, &field_key);
            wrap_message(field, inner)
        }

        Rule::MaxSize(n) => {
            let element_type = vec_inner_type_from_field(field);
            let inner = emit_vec_validator(field, quote! {
                ::nebula_validator::validators::max_size::<#element_type>(#n)
            }, &field_key);
            wrap_message(field, inner)
        }

        Rule::ExactSize(n) => {
            let element_type = vec_inner_type_from_field(field);
            let inner = emit_vec_validator(field, quote! {
                ::nebula_validator::validators::exact_size::<#element_type>(#n)
            }, &field_key);
            wrap_message(field, inner)
        }

        Rule::SizeRange { min, max } => {
            let element_type = vec_inner_type_from_field(field);
            let inner = emit_vec_validator(field, quote! {
                ::nebula_validator::validators::size_range::<#element_type>(#min, #max)
            }, &field_key);
            wrap_message(field, inner)
        }

        Rule::NotEmptyCollection => {
            let element_type = vec_inner_type_from_field(field);
            let inner = emit_vec_validator(field, quote! {
                ::nebula_validator::validators::not_empty_collection::<#element_type>()
            }, &field_key);
            wrap_message(field, inner)
        }

        Rule::StringFormat(format) => {
            let validator_expr = string_format_to_tokens(format);
            let inner = emit_str_validator(field, validator_expr, &field_key);
            wrap_message(field, inner)
        }

        Rule::StringFactory { kind, arg } => {
            let validator_expr = match kind {
                StringFactoryKind::Contains => quote!(::nebula_validator::validators::contains(#arg)),
                StringFactoryKind::StartsWith => quote!(::nebula_validator::validators::starts_with(#arg)),
                StringFactoryKind::EndsWith => quote!(::nebula_validator::validators::ends_with(#arg)),
            };
            let inner = emit_str_validator(field, validator_expr, &field_key);
            wrap_message(field, inner)
        }

        Rule::IsTrue => {
            let inner = emit_bool_validator(field, quote!(::nebula_validator::validators::is_true()), &field_key);
            wrap_message(field, inner)
        }

        Rule::IsFalse => {
            let inner = emit_bool_validator(field, quote!(::nebula_validator::validators::is_false()), &field_key);
            wrap_message(field, inner)
        }

        Rule::Regex(pattern) => {
            let inner = emit_regex_validator(field, pattern, &field_key);
            wrap_message(field, inner)
        }

        Rule::Nested => {
            let inner = emit_nested_validator(field, &field_key);
            wrap_message(field, inner)
        }

        Rule::Custom(expr) => {
            let inner = emit_custom_validator(field, expr, &field_key);
            wrap_message(field, inner)
        }
    }
}

// ---------------------------------------------------------------------------
// Shared codegen helpers — each called from ONE place
// ---------------------------------------------------------------------------

fn emit_str_validator(field: &FieldDef, validator_expr: TokenStream2, field_key: &str) -> TokenStream2 {
    let field_name = &field.ident;
    let inner = if field.is_option {
        quote! {
            if let Err(e) = ::nebula_validator::foundation::Validate::validate(&#validator_expr, value.as_str()) {
                errors.add(e.with_field(#field_key));
            }
        }
    } else {
        quote! {
            if let Err(e) = ::nebula_validator::foundation::Validate::validate(&#validator_expr, input.#field_name.as_str()) {
                errors.add(e.with_field(#field_key));
            }
        }
    };
    wrap_option(field, inner)
}

fn emit_bool_validator(field: &FieldDef, validator_expr: TokenStream2, field_key: &str) -> TokenStream2 {
    let field_name = &field.ident;
    let inner = if field.is_option {
        quote! {
            if let Err(e) = ::nebula_validator::foundation::Validate::validate(&#validator_expr, value) {
                errors.add(e.with_field(#field_key));
            }
        }
    } else {
        quote! {
            if let Err(e) = ::nebula_validator::foundation::Validate::validate(&#validator_expr, &input.#field_name) {
                errors.add(e.with_field(#field_key));
            }
        }
    };
    wrap_option(field, inner)
}

fn emit_vec_validator(field: &FieldDef, validator_expr: TokenStream2, field_key: &str) -> TokenStream2 {
    let field_name = &field.ident;
    let inner = if field.is_option {
        quote! {
            if let Err(e) = ::nebula_validator::foundation::Validate::validate(
                &#validator_expr,
                value.as_slice(),
            ) {
                errors.add(e.with_field(#field_key));
            }
        }
    } else {
        quote! {
            if let Err(e) = ::nebula_validator::foundation::Validate::validate(
                &#validator_expr,
                input.#field_name.as_slice(),
            ) {
                errors.add(e.with_field(#field_key));
            }
        }
    };
    wrap_option(field, inner)
}

fn emit_regex_validator(field: &FieldDef, pattern: &str, field_key: &str) -> TokenStream2 {
    let field_name = &field.ident;
    let inner = if field.is_option {
        quote! {
            match ::nebula_validator::validators::matches_regex(#pattern) {
                Ok(v) => {
                    if let Err(e) = ::nebula_validator::foundation::Validate::validate(&v, value.as_str()) {
                        errors.add(e.with_field(#field_key));
                    }
                }
                Err(e) => {
                    errors.add(
                        ::nebula_validator::foundation::ValidationError::new(
                            "invalid_regex_pattern",
                            format!("invalid regex pattern `{}`: {}", #pattern, e),
                        ).with_field(#field_key),
                    );
                }
            }
        }
    } else {
        quote! {
            match ::nebula_validator::validators::matches_regex(#pattern) {
                Ok(v) => {
                    if let Err(e) = ::nebula_validator::foundation::Validate::validate(&v, input.#field_name.as_str()) {
                        errors.add(e.with_field(#field_key));
                    }
                }
                Err(e) => {
                    errors.add(
                        ::nebula_validator::foundation::ValidationError::new(
                            "invalid_regex_pattern",
                            format!("invalid regex pattern `{}`: {}", #pattern, e),
                        ).with_field(#field_key),
                    );
                }
            }
        }
    };
    wrap_option(field, inner)
}

fn emit_nested_validator(field: &FieldDef, field_key: &str) -> TokenStream2 {
    let field_name = &field.ident;
    let inner = if field.is_option {
        quote! {
            if let Err(e) = ::nebula_validator::combinators::SelfValidating::check(value) {
                errors.add(e.with_field(#field_key));
            }
        }
    } else {
        quote! {
            if let Err(e) = ::nebula_validator::combinators::SelfValidating::check(&input.#field_name) {
                errors.add(e.with_field(#field_key));
            }
        }
    };
    wrap_option(field, inner)
}

fn emit_custom_validator(field: &FieldDef, expr: &TokenStream2, field_key: &str) -> TokenStream2 {
    let field_name = &field.ident;
    let inner = if field.is_option {
        quote! {
            if let Err(e) = (#expr)(value) {
                errors.add(e.with_field(#field_key));
            }
        }
    } else {
        quote! {
            if let Err(e) = (#expr)(&input.#field_name) {
                errors.add(e.with_field(#field_key));
            }
        }
    };
    wrap_option(field, inner)
}

// ---------------------------------------------------------------------------
// each() loop emission
// ---------------------------------------------------------------------------

fn emit_each_loop(field: &FieldDef, each: &EachRules) -> TokenStream2 {
    let field_name = &field.ident;
    let field_key = field_name.to_string();

    let each_checks: Vec<TokenStream2> = each.rules.iter().map(|rule| {
        emit_each_rule(rule, &field_key, &each.element_ty)
    }).collect();

    let loop_body = quote! {
        for (index, value) in collection.iter().enumerate() {
            let each_field = format!("{}[{}]", #field_key, index);
            #(#each_checks)*
        }
    };

    let is_option = option_type_check(&field.ty);
    if is_option {
        quote! {
            if let Some(collection) = input.#field_name.as_ref() {
                #loop_body
            }
        }
    } else {
        quote! {
            let collection = &input.#field_name;
            #loop_body
        }
    }
}

/// Emit a single rule for an element inside each().
/// Here `value` is the loop variable (element ref), `each_field` is the indexed field path.
fn emit_each_rule(rule: &Rule, field_key: &str, element_ty: &syn::Type) -> TokenStream2 {
    let is_string = is_string_type_check(element_ty);

    match rule {
        Rule::MinLength(n) => quote! {
            if value.len() < #n {
                errors.add(::nebula_validator::foundation::ValidationError::min_length(
                    each_field.clone(), #n, value.len(),
                ));
            }
        },
        Rule::MaxLength(n) => quote! {
            if value.len() > #n {
                errors.add(::nebula_validator::foundation::ValidationError::max_length(
                    each_field.clone(), #n, value.len(),
                ));
            }
        },
        Rule::ExactLength(n) => quote! {
            if value.len() != #n {
                errors.add(::nebula_validator::foundation::ValidationError::exact_length(
                    each_field.clone(), #n, value.len(),
                ));
            }
        },
        Rule::Min(bound) => quote! {
            if value < &#bound {
                errors.add(
                    ::nebula_validator::foundation::ValidationError::new(
                        "min", format!("{} must be >= {}", each_field, #bound),
                    ).with_field(each_field.clone()),
                );
            }
        },
        Rule::Max(bound) => quote! {
            if value > &#bound {
                errors.add(
                    ::nebula_validator::foundation::ValidationError::new(
                        "max", format!("{} must be <= {}", each_field, #bound),
                    ).with_field(each_field.clone()),
                );
            }
        },
        Rule::StringFormat(format) => {
            let expr = string_format_to_tokens(format);
            quote! {
                if let Err(e) = ::nebula_validator::foundation::Validate::validate(&#expr, value.as_str()) {
                    errors.add(e.with_field(each_field.clone()));
                }
            }
        }
        Rule::StringFactory { kind, arg } => {
            let expr = match kind {
                StringFactoryKind::Contains => quote!(::nebula_validator::validators::contains(#arg)),
                StringFactoryKind::StartsWith => quote!(::nebula_validator::validators::starts_with(#arg)),
                StringFactoryKind::EndsWith => quote!(::nebula_validator::validators::ends_with(#arg)),
            };
            quote! {
                if let Err(e) = ::nebula_validator::foundation::Validate::validate(&#expr, value.as_str()) {
                    errors.add(e.with_field(each_field.clone()));
                }
            }
        }
        Rule::Regex(pattern) => quote! {
            match ::nebula_validator::validators::matches_regex(#pattern) {
                Ok(v) => {
                    if let Err(e) = ::nebula_validator::foundation::Validate::validate(&v, value.as_str()) {
                        errors.add(e.with_field(each_field.clone()));
                    }
                }
                Err(e) => {
                    errors.add(
                        ::nebula_validator::foundation::ValidationError::new(
                            "invalid_regex_pattern",
                            format!("invalid regex pattern `{}`: {}", #pattern, e),
                        ).with_field(each_field.clone()),
                    );
                }
            }
        },
        Rule::Nested => quote! {
            if let Err(e) = ::nebula_validator::combinators::SelfValidating::check(value) {
                errors.add(e.with_field(each_field.clone()));
            }
        },
        Rule::Custom(expr) => quote! {
            if let Err(e) = (#expr)(value) {
                errors.add(e.with_field(each_field.clone()));
            }
        },
        // These rules don't apply inside each()
        Rule::Required | Rule::LengthRange { .. } | Rule::MinSize(_) | Rule::MaxSize(_)
        | Rule::ExactSize(_) | Rule::SizeRange { .. } | Rule::NotEmptyCollection
        | Rule::IsTrue | Rule::IsFalse => TokenStream2::new(),
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn string_format_to_tokens(format: &StringFormat) -> TokenStream2 {
    match format {
        StringFormat::NotEmpty => quote!(::nebula_validator::validators::not_empty()),
        StringFormat::Alphanumeric => quote!(::nebula_validator::validators::alphanumeric()),
        StringFormat::Alphabetic => quote!(::nebula_validator::validators::alphabetic()),
        StringFormat::Numeric => quote!(::nebula_validator::validators::numeric()),
        StringFormat::Lowercase => quote!(::nebula_validator::validators::lowercase()),
        StringFormat::Uppercase => quote!(::nebula_validator::validators::uppercase()),
        StringFormat::Email => quote!(::nebula_validator::validators::email()),
        StringFormat::Url => quote!(::nebula_validator::validators::url()),
        StringFormat::Ipv4 => quote!(::nebula_validator::validators::ipv4()),
        StringFormat::Ipv6 => quote!(::nebula_validator::validators::ipv6()),
        StringFormat::IpAddr => quote!(::nebula_validator::validators::ip_addr()),
        StringFormat::Hostname => quote!(::nebula_validator::validators::hostname()),
        StringFormat::Uuid => quote!(::nebula_validator::validators::uuid()),
        StringFormat::Date => quote!(::nebula_validator::validators::date()),
        StringFormat::DateTime => quote!(::nebula_validator::validators::date_time()),
        StringFormat::Time => quote!(::nebula_validator::validators::time()),
    }
}

/// Extract Vec<T>'s inner type from a FieldDef (assumes parse already validated this is Vec).
fn vec_inner_type_from_field(field: &FieldDef) -> &syn::Type {
    let ty = &field.inner_ty;
    vec_inner_type_recursive(ty).unwrap_or(ty)
}

fn vec_inner_type_recursive(ty: &syn::Type) -> Option<&syn::Type> {
    let syn::Type::Path(type_path) = ty else { return None };
    let segment = type_path.path.segments.last()?;
    if segment.ident != "Vec" { return None }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else { return None };
    let syn::GenericArgument::Type(inner) = args.args.first()? else { return None };
    Some(inner)
}

fn option_type_check(ty: &syn::Type) -> bool {
    let syn::Type::Path(type_path) = ty else { return false };
    type_path.path.segments.last().is_some_and(|s| s.ident == "Option")
}

fn is_string_type_check(ty: &syn::Type) -> bool {
    let syn::Type::Path(type_path) = ty else { return false };
    type_path.path.segments.last().is_some_and(|s| s.ident == "String")
}
```

**Step 2: Register the module**

In `lib.rs`, add `mod emit;`.

**Step 3: Run check**

Run: `rtk cargo check -p nebula-validator-macros`
Expected: PASS

**Step 4: Commit**

```bash
rtk git add crates/validator/macros/src/emit.rs crates/validator/macros/src/lib.rs
rtk git commit -m "refactor(validator-macros): add emit module — IR to TokenStream codegen"
```

---

### Task 4: Wire up the 3-phase pipeline and delete old code

**Files:**
- Modify: `crates/validator/macros/src/lib.rs`
- Modify: `crates/validator/macros/src/validator.rs` (replace all contents)

**Step 1: Rewrite `validator.rs` to use the pipeline**

Replace the entire contents of `validator.rs`:

```rust
//! Validator derive macro implementation — 3-phase pipeline.

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

use nebula_macro_support::diag;

use crate::{emit, parse};

/// Entry point for `#[derive(Validator)]`.
pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand(input) {
        Ok(ts) => ts.into(),
        Err(e) => diag::to_compile_error(e).into(),
    }
}

fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    // Phase 1: Parse attributes into IR
    let ir = parse::parse(&input)?;

    // Phase 2: (semantic checks are done during parsing for now)

    // Phase 3: Generate code from IR
    Ok(emit::emit(&ir))
}
```

**Step 2: Update `lib.rs` to declare all modules**

```rust
//! Proc-macro crate for the `Validator` derive macro.
//!
//! Implements `nebula_validator::foundation::Validate` for the struct and
//! generates an inherent `validate_fields()` helper.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;

use proc_macro::TokenStream;

mod emit;
mod model;
mod parse;
mod validator;

/// Derive macro for generating field-based validators.
///
/// See the `nebula-macros` crate documentation for the full list of
/// supported container and field attributes.
#[proc_macro_derive(Validator, attributes(validator, validate))]
pub fn derive_validator(input: TokenStream) -> TokenStream {
    validator::derive(input)
}
```

**Step 3: Run check**

Run: `rtk cargo check -p nebula-validator-macros`
Expected: PASS — fix any compile errors from the migration.

**Step 4: Run full workspace check**

Run: `rtk cargo check --workspace`
Expected: PASS

**Step 5: Commit**

```bash
rtk git add crates/validator/macros/src/validator.rs crates/validator/macros/src/lib.rs
rtk git commit -m "refactor(validator-macros): wire 3-phase pipeline, replace monolithic expand()"
```

---

### Task 5: Add compile-test assertions

**Files:**
- Create: `crates/validator/macros/tests/derive_tests.rs`

**Step 1: Write integration tests that exercise the derive macro**

These tests verify the macro produces correct validation behavior. They cover:
- Basic field validation (min_length, max_length, required)
- Option wrapping
- String format validators
- Bool validators
- Nested validation
- Custom validators
- `each()` element validation
- Message override

```rust
//! Integration tests for the Validator derive macro.

use nebula_validator::foundation::{Validate, ValidationErrors};
use nebula_validator::combinators::SelfValidating;

// Pull in the derive macro
use nebula_validator::Validator;

#[derive(Validator)]
#[validator(message = "user validation failed")]
struct User {
    #[validate(required, min_length = 3, max_length = 50)]
    name: Option<String>,

    #[validate(email)]
    email: String,

    #[validate(min = 0, max = 150)]
    age: u32,

    #[validate(is_true)]
    accepted_terms: bool,
}

#[test]
fn valid_user_passes() {
    let user = User {
        name: Some("Alice".to_string()),
        email: "alice@example.com".to_string(),
        age: 30,
        accepted_terms: true,
    };
    assert!(user.validate_fields().is_ok());
}

#[test]
fn missing_required_name_fails() {
    let user = User {
        name: None,
        email: "alice@example.com".to_string(),
        age: 30,
        accepted_terms: true,
    };
    let err = user.validate_fields().unwrap_err();
    assert!(err.has_errors());
}

#[test]
fn short_name_fails() {
    let user = User {
        name: Some("Al".to_string()),
        email: "alice@example.com".to_string(),
        age: 30,
        accepted_terms: true,
    };
    assert!(user.validate_fields().is_err());
}

#[test]
fn invalid_email_fails() {
    let user = User {
        name: Some("Alice".to_string()),
        email: "not-an-email".to_string(),
        age: 30,
        accepted_terms: true,
    };
    assert!(user.validate_fields().is_err());
}

#[test]
fn age_over_max_fails() {
    let user = User {
        name: Some("Alice".to_string()),
        email: "alice@example.com".to_string(),
        age: 200,
        accepted_terms: true,
    };
    assert!(user.validate_fields().is_err());
}

#[test]
fn unaccepted_terms_fails() {
    let user = User {
        name: Some("Alice".to_string()),
        email: "alice@example.com".to_string(),
        age: 30,
        accepted_terms: false,
    };
    assert!(user.validate_fields().is_err());
}

#[test]
fn self_validating_works() {
    let user = User {
        name: Some("Alice".to_string()),
        email: "alice@example.com".to_string(),
        age: 30,
        accepted_terms: true,
    };
    assert!(SelfValidating::check(&user).is_ok());
}

#[test]
fn validate_trait_works() {
    let user = User {
        name: Some("Alice".to_string()),
        email: "alice@example.com".to_string(),
        age: 30,
        accepted_terms: true,
    };
    assert!(user.validate(&user).is_ok());
}

// --- Vec + each() tests ---

#[derive(Validator)]
struct TaggedItem {
    #[validate(not_empty_collection, each(not_empty, min_length = 1))]
    tags: Vec<String>,
}

#[test]
fn valid_tags_pass() {
    let item = TaggedItem {
        tags: vec!["rust".to_string(), "validator".to_string()],
    };
    assert!(item.validate_fields().is_ok());
}

#[test]
fn empty_collection_fails() {
    let item = TaggedItem {
        tags: vec![],
    };
    assert!(item.validate_fields().is_err());
}

#[test]
fn empty_tag_element_fails() {
    let item = TaggedItem {
        tags: vec!["rust".to_string(), "".to_string()],
    };
    assert!(item.validate_fields().is_err());
}

// --- Nested validation ---

#[derive(Validator)]
struct Address {
    #[validate(not_empty)]
    city: String,
}

#[derive(Validator)]
struct Person {
    #[validate(not_empty)]
    name: String,

    #[validate(nested)]
    address: Address,
}

#[test]
fn valid_nested_passes() {
    let person = Person {
        name: "Bob".to_string(),
        address: Address { city: "Portland".to_string() },
    };
    assert!(person.validate_fields().is_ok());
}

#[test]
fn invalid_nested_fails() {
    let person = Person {
        name: "Bob".to_string(),
        address: Address { city: "".to_string() },
    };
    assert!(person.validate_fields().is_err());
}

// --- Message override ---

#[derive(Validator)]
struct WithMessage {
    #[validate(min_length = 5, message = "name is too short")]
    name: String,
}

#[test]
fn message_override_applied() {
    let w = WithMessage { name: "ab".to_string() };
    let err = w.validate_fields().unwrap_err();
    let errors: Vec<_> = err.errors().collect();
    assert!(!errors.is_empty());
    assert_eq!(errors[0].message.as_ref(), "name is too short");
}

// --- Custom validator ---

fn validate_even(value: &u32) -> Result<(), nebula_validator::foundation::ValidationError> {
    if value % 2 == 0 {
        Ok(())
    } else {
        Err(nebula_validator::foundation::ValidationError::new("even", "must be even"))
    }
}

#[derive(Validator)]
struct EvenNumber {
    #[validate(custom = validate_even)]
    value: u32,
}

#[test]
fn custom_validator_passes() {
    let n = EvenNumber { value: 4 };
    assert!(n.validate_fields().is_ok());
}

#[test]
fn custom_validator_fails() {
    let n = EvenNumber { value: 3 };
    assert!(n.validate_fields().is_err());
}
```

**Step 2: Run the tests**

Run: `rtk cargo nextest run -p nebula-validator`
Expected: All tests PASS

**Step 3: Run full workspace validation**

Run: `rtk cargo fmt && rtk cargo clippy --workspace -- -D warnings && rtk cargo nextest run --workspace`
Expected: PASS

**Step 4: Commit**

```bash
rtk git add crates/validator/macros/tests/derive_tests.rs
rtk git commit -m "test(validator-macros): add integration tests for derive macro"
```

---

### Task 6: Cleanup — remove dead code from `validation_codegen.rs`

**Files:**
- Modify: `crates/sdk/macros-support/src/validation_codegen.rs`

**Step 1: Check what's still used**

Run: `rtk cargo check --workspace` to identify any remaining callers.

The `config/macros/src/config.rs` still uses some `validation_codegen` helpers. Only remove functions NOT used by config or any other crate. Specifically:

- Functions still used by `config.rs`: `is_option_type`, `parse_usize`, `parse_number_lit`, `generate_len_check`, `generate_cmp_check`, `generate_str_validator_check`, `generate_regex_validator_check`, `built_in_string_validator_flags`, `value_token`
- Functions only used by old `validator.rs` (now dead): `generate_exact_len_check`, `built_in_string_validator_factories`

Remove dead functions and add `#[allow(dead_code)]` if needed for functions only used by one consumer.

**Step 2: Run workspace check**

Run: `rtk cargo check --workspace`
Expected: PASS — no dead code warnings

**Step 3: Commit**

```bash
rtk git add crates/sdk/macros-support/src/validation_codegen.rs
rtk git commit -m "chore(macro-support): remove validation_codegen functions superseded by validator IR"
```

---

### Task 7: Update context file

**Files:**
- Modify: `.claude/crates/validator.md`

**Step 1: Add a note about the macro architecture**

Add to the validator context file:

```markdown
### Derive Macro Architecture (validator-macros)
- 3-phase pipeline: `parse.rs` → (check) → `emit.rs`
- IR types in `model.rs`: `ValidatorInput`, `FieldDef`, `Rule` enum, `EachRules`
- Option-wrapping centralized in `emit::wrap_option()`
- Message override centralized in `emit::wrap_message()`
- `validation_codegen.rs` helpers still used by `config-macros`; validator-macros uses its own IR
```

**Step 2: Commit**

```bash
rtk git add .claude/crates/validator.md
rtk git commit -m "docs(validator): update context file with macro architecture"
```

---

## Summary: Before → After

| Metric | Before | After |
|--------|--------|-------|
| Files | 1 (`validator.rs`) | 4 (`model.rs`, `parse.rs`, `emit.rs`, `validator.rs`) |
| `expand()` function | 1050 lines | 15 lines (pipeline call) |
| Message override code | ~20 copies | 1 (`wrap_message`) |
| Option wrapping code | ~15 copies | 1 (`wrap_option`) |
| `each()` validator duplication | Full copy of field logic | Reuses `Rule` enum |
| IR | None | `ValidatorInput` → `FieldDef` → `Rule` enum |
| Testability | Untestable (all inline) | IR types can be unit-tested separately |
