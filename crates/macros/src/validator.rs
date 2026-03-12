use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Type, parse_macro_input};

use crate::support::validation_codegen::{
    built_in_string_validator_factories, built_in_string_validator_flags, generate_cmp_check,
    generate_exact_len_check, generate_len_check, generate_regex_validator_check,
    generate_str_validator_check, is_option_type, parse_number_lit, parse_usize,
};
use crate::support::{attrs, diag};

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

fn option_inner_type(ty: &Type) -> Option<&Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    let syn::GenericArgument::Type(inner) = args.args.first()? else {
        return None;
    };
    Some(inner)
}

fn vec_inner_type(ty: &Type) -> Option<&Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident != "Vec" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    let syn::GenericArgument::Type(inner) = args.args.first()? else {
        return None;
    };
    Some(inner)
}

fn is_string_type(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    type_path
        .path
        .segments
        .last()
        .map(|segment| segment.ident == "String")
        .unwrap_or(false)
}

fn is_bool_type(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    type_path
        .path
        .segments
        .last()
        .map(|segment| segment.ident == "bool")
        .unwrap_or(false)
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
            let parsed = syn::parse2::<syn::Expr>(tokens.clone()).map_err(|e| {
                diag::error_spanned(tokens, format!("invalid each(...) entry: {e}"))
            })?;
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
                    path.path
                        .segments
                        .into_iter()
                        .next()
                        .expect("segment")
                        .ident,
                ))
            } else {
                Err(diag::error_spanned(
                    &value_tokens(span_source),
                    "each(...) flags must be single identifiers",
                ))
            }
        }
        syn::Expr::Assign(assign) => {
            let syn::Expr::Path(left_path) = *assign.left else {
                return Err(diag::error_spanned(
                    &value_tokens(span_source),
                    "each(...) key-value entry must use identifier keys",
                ));
            };
            if left_path.path.segments.len() != 1 || left_path.path.leading_colon.is_some() {
                return Err(diag::error_spanned(
                    &value_tokens(span_source),
                    "each(...) key-value entry must use identifier keys",
                ));
            }
            let key = left_path
                .path
                .segments
                .into_iter()
                .next()
                .expect("segment")
                .ident;
            let value = expr_to_attr_value(*assign.right);
            Ok(attrs::AttrItem::KeyValue { key, value })
        }
        _ => Err(diag::error_spanned(
            &value_tokens(span_source),
            "unsupported each(...) entry; use flags or key-value entries",
        )),
    }
}

fn expr_to_attr_value(expr: syn::Expr) -> attrs::AttrValue {
    match expr {
        syn::Expr::Path(path) => {
            if path.path.segments.len() == 1 && path.path.leading_colon.is_none() {
                attrs::AttrValue::Ident(
                    path.path
                        .segments
                        .into_iter()
                        .next()
                        .expect("segment")
                        .ident,
                )
            } else {
                attrs::AttrValue::Tokens(quote!(#path))
            }
        }
        syn::Expr::Lit(lit) => attrs::AttrValue::Lit(lit.lit),
        other => attrs::AttrValue::Tokens(quote!(#other)),
    }
}

fn value_tokens(value: &attrs::AttrValue) -> TokenStream2 {
    match value {
        attrs::AttrValue::Ident(ident) => quote!(#ident),
        attrs::AttrValue::Lit(lit) => quote!(#lit),
        attrs::AttrValue::Tokens(tokens) => tokens.clone(),
    }
}

fn parse_each_args(validate_attrs: &attrs::AttrArgs) -> syn::Result<Option<attrs::AttrArgs>> {
    let Some(values) = validate_attrs.get_list_values("each") else {
        return Ok(None);
    };

    let mut items = Vec::with_capacity(values.len());
    for value in values {
        items.push(parse_list_item_to_attr_item(value)?);
    }

    Ok(Some(attrs::AttrArgs { items }))
}

fn parse_min_max_list(
    validate_attrs: &attrs::AttrArgs,
    key: &str,
) -> syn::Result<Option<(usize, usize)>> {
    let Some(values) = validate_attrs.get_list_values(key) else {
        return Ok(None);
    };

    let mut min: Option<usize> = None;
    let mut max: Option<usize> = None;

    for value in values {
        let item = parse_list_item_to_attr_item(value)?;
        let attrs::AttrItem::KeyValue {
            key: entry_key,
            value,
        } = item
        else {
            return Err(diag::error_spanned(
                &value_tokens(value),
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
                    &value_tokens(&other),
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
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("`{key}` requires both `min` and `max`"),
        )
    })?;
    let max = max.ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("`{key}` requires both `min` and `max`"),
        )
    })?;

    if min > max {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("`{key}` requires `min <= max`"),
        ));
    }

    Ok(Some((min, max)))
}

pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand(input) {
        Ok(ts) => ts,
        Err(e) => diag::to_compile_error(e),
    }
}

#[expect(
    clippy::excessive_nesting,
    reason = "derive codegen branches by attribute kind and message override; flattening harms macro readability"
)]
fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            syn::Fields::Named(fields) => &fields.named,
            _ => {
                return Err(syn::Error::new(
                    struct_name.span(),
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
    let root_message = validator_attrs
        .get_string("message")
        .unwrap_or_else(|| "validation failed".to_string());

    let mut checks = Vec::new();
    for field in fields {
        let field_name = match &field.ident {
            Some(name) => name,
            None => continue,
        };
        let validate_attrs = attrs::parse_attrs(&field.attrs, "validate")?;
        let each_attrs = parse_each_args(&validate_attrs)?;

        let is_option = is_option_type(&field.ty);
        let field_value_type = option_inner_type(&field.ty).unwrap_or(&field.ty);
        let collection_element_type = vec_inner_type(field_value_type);
        let field_is_vec = collection_element_type.is_some();
        let field_is_string = is_string_type(field_value_type);
        let field_is_bool = is_bool_type(field_value_type);
        let mut field_checks = Vec::new();
        let field_key = field_name.to_string();
        let field_message = validate_attrs.get_string("message");

        if validate_attrs.has_flag("required") && is_option {
            if let Some(message) = &field_message {
                field_checks.push(quote! {
                    if input.#field_name.is_none() {
                        let mut err = ::nebula_validator::foundation::ValidationError::required(#field_key);
                        err.message = ::std::borrow::Cow::Owned(#message.to_string());
                        errors.add(err);
                    }
                });
            } else {
                field_checks.push(quote! {
                    if input.#field_name.is_none() {
                        errors.add(::nebula_validator::foundation::ValidationError::required(#field_key));
                    }
                });
            }
        }

        if let Some(min_len) = parse_usize(&validate_attrs, "min_length")? {
            let check = generate_len_check(field_name, &field_key, is_option, min_len, true);
            if let Some(message) = &field_message {
                field_checks.push(quote! {
                    let before = errors.len();
                    #check
                    let after = errors.len();
                    if after > before {
                        if let Some(last) = errors.last_mut() {
                            last.message = ::std::borrow::Cow::Owned(#message.to_string());
                        }
                    }
                });
            } else {
                field_checks.push(check);
            }
        }

        if let Some(max_len) = parse_usize(&validate_attrs, "max_length")? {
            let check = generate_len_check(field_name, &field_key, is_option, max_len, false);
            if let Some(message) = &field_message {
                field_checks.push(quote! {
                    let before = errors.len();
                    #check
                    let after = errors.len();
                    if after > before {
                        if let Some(last) = errors.last_mut() {
                            last.message = ::std::borrow::Cow::Owned(#message.to_string());
                        }
                    }
                });
            } else {
                field_checks.push(check);
            }
        }

        if let Some(exact_len) = parse_usize(&validate_attrs, "exact_length")? {
            let check = generate_exact_len_check(field_name, &field_key, is_option, exact_len);
            if let Some(message) = &field_message {
                field_checks.push(quote! {
                    let before = errors.len();
                    #check
                    let after = errors.len();
                    if after > before {
                        if let Some(last) = errors.last_mut() {
                            last.message = ::std::borrow::Cow::Owned(#message.to_string());
                        }
                    }
                });
            } else {
                field_checks.push(check);
            }
        }

        if let Some((min_len, max_len)) = parse_min_max_list(&validate_attrs, "length_range")? {
            if !field_is_string {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "`length_range(...)` requires `String` or `Option<String>` fields",
                ));
            }

            let check = if is_option {
                quote! {
                    if let Some(value) = input.#field_name.as_ref() {
                        match ::nebula_validator::validators::length_range(#min_len, #max_len) {
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
                }
            } else {
                quote! {
                    match ::nebula_validator::validators::length_range(#min_len, #max_len) {
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

            if let Some(message) = &field_message {
                field_checks.push(quote! {
                    let before = errors.len();
                    #check
                    let after = errors.len();
                    if after > before {
                        if let Some(last) = errors.last_mut() {
                            last.message = ::std::borrow::Cow::Owned(#message.to_string());
                        }
                    }
                });
            } else {
                field_checks.push(check);
            }
        }

        if let Some(min_value) = parse_number_lit(&validate_attrs, "min")? {
            let check = generate_cmp_check(field_name, &field_key, is_option, min_value, true);
            if let Some(message) = &field_message {
                field_checks.push(quote! {
                    let before = errors.len();
                    #check
                    let after = errors.len();
                    if after > before {
                        if let Some(last) = errors.last_mut() {
                            last.message = ::std::borrow::Cow::Owned(#message.to_string());
                        }
                    }
                });
            } else {
                field_checks.push(check);
            }
        }

        if let Some(max_value) = parse_number_lit(&validate_attrs, "max")? {
            let check = generate_cmp_check(field_name, &field_key, is_option, max_value, false);
            if let Some(message) = &field_message {
                field_checks.push(quote! {
                    let before = errors.len();
                    #check
                    let after = errors.len();
                    if after > before {
                        if let Some(last) = errors.last_mut() {
                            last.message = ::std::borrow::Cow::Owned(#message.to_string());
                        }
                    }
                });
            } else {
                field_checks.push(check);
            }
        }

        if let Some(min_size) = parse_usize(&validate_attrs, "min_size")? {
            let Some(element_type) = collection_element_type else {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "`min_size` requires `Vec<T>` or `Option<Vec<T>>` fields",
                ));
            };

            let check = if is_option {
                quote! {
                    if let Some(value) = input.#field_name.as_ref() {
                        if let Err(e) = ::nebula_validator::foundation::Validate::validate(
                            &::nebula_validator::validators::min_size::<#element_type>(#min_size),
                            value.as_slice(),
                        ) {
                            errors.add(e.with_field(#field_key));
                        }
                    }
                }
            } else {
                quote! {
                    if let Err(e) = ::nebula_validator::foundation::Validate::validate(
                        &::nebula_validator::validators::min_size::<#element_type>(#min_size),
                        input.#field_name.as_slice(),
                    ) {
                        errors.add(e.with_field(#field_key));
                    }
                }
            };

            if let Some(message) = &field_message {
                field_checks.push(quote! {
                    let before = errors.len();
                    #check
                    let after = errors.len();
                    if after > before {
                        if let Some(last) = errors.last_mut() {
                            last.message = ::std::borrow::Cow::Owned(#message.to_string());
                        }
                    }
                });
            } else {
                field_checks.push(check);
            }
        }

        if let Some(max_size) = parse_usize(&validate_attrs, "max_size")? {
            let Some(element_type) = collection_element_type else {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "`max_size` requires `Vec<T>` or `Option<Vec<T>>` fields",
                ));
            };

            let check = if is_option {
                quote! {
                    if let Some(value) = input.#field_name.as_ref() {
                        if let Err(e) = ::nebula_validator::foundation::Validate::validate(
                            &::nebula_validator::validators::max_size::<#element_type>(#max_size),
                            value.as_slice(),
                        ) {
                            errors.add(e.with_field(#field_key));
                        }
                    }
                }
            } else {
                quote! {
                    if let Err(e) = ::nebula_validator::foundation::Validate::validate(
                        &::nebula_validator::validators::max_size::<#element_type>(#max_size),
                        input.#field_name.as_slice(),
                    ) {
                        errors.add(e.with_field(#field_key));
                    }
                }
            };

            if let Some(message) = &field_message {
                field_checks.push(quote! {
                    let before = errors.len();
                    #check
                    let after = errors.len();
                    if after > before {
                        if let Some(last) = errors.last_mut() {
                            last.message = ::std::borrow::Cow::Owned(#message.to_string());
                        }
                    }
                });
            } else {
                field_checks.push(check);
            }
        }

        if let Some(exact_size) = parse_usize(&validate_attrs, "exact_size")? {
            let Some(element_type) = collection_element_type else {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "`exact_size` requires `Vec<T>` or `Option<Vec<T>>` fields",
                ));
            };

            let check = if is_option {
                quote! {
                    if let Some(value) = input.#field_name.as_ref() {
                        if let Err(e) = ::nebula_validator::foundation::Validate::validate(
                            &::nebula_validator::validators::exact_size::<#element_type>(#exact_size),
                            value.as_slice(),
                        ) {
                            errors.add(e.with_field(#field_key));
                        }
                    }
                }
            } else {
                quote! {
                    if let Err(e) = ::nebula_validator::foundation::Validate::validate(
                        &::nebula_validator::validators::exact_size::<#element_type>(#exact_size),
                        input.#field_name.as_slice(),
                    ) {
                        errors.add(e.with_field(#field_key));
                    }
                }
            };

            if let Some(message) = &field_message {
                field_checks.push(quote! {
                    let before = errors.len();
                    #check
                    let after = errors.len();
                    if after > before {
                        if let Some(last) = errors.last_mut() {
                            last.message = ::std::borrow::Cow::Owned(#message.to_string());
                        }
                    }
                });
            } else {
                field_checks.push(check);
            }
        }

        if validate_attrs.has_flag("not_empty_collection") {
            if !field_is_vec {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "`not_empty_collection` requires `Vec<T>` or `Option<Vec<T>>` fields",
                ));
            }

            let element_type = collection_element_type.expect("vector element type checked");
            let check = if is_option {
                quote! {
                    if let Some(value) = input.#field_name.as_ref() {
                        if let Err(e) = ::nebula_validator::foundation::Validate::validate(
                            &::nebula_validator::validators::not_empty_collection::<#element_type>(),
                            value.as_slice(),
                        ) {
                            errors.add(e.with_field(#field_key));
                        }
                    }
                }
            } else {
                quote! {
                    if let Err(e) = ::nebula_validator::foundation::Validate::validate(
                        &::nebula_validator::validators::not_empty_collection::<#element_type>(),
                        input.#field_name.as_slice(),
                    ) {
                        errors.add(e.with_field(#field_key));
                    }
                }
            };

            if let Some(message) = &field_message {
                field_checks.push(quote! {
                    let before = errors.len();
                    #check
                    let after = errors.len();
                    if after > before {
                        if let Some(last) = errors.last_mut() {
                            last.message = ::std::borrow::Cow::Owned(#message.to_string());
                        }
                    }
                });
            } else {
                field_checks.push(check);
            }
        }

        if let Some((min_size, max_size)) = parse_min_max_list(&validate_attrs, "size_range")? {
            let Some(element_type) = collection_element_type else {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "`size_range(...)` requires `Vec<T>` or `Option<Vec<T>>` fields",
                ));
            };

            let check = if is_option {
                quote! {
                    if let Some(value) = input.#field_name.as_ref() {
                        if let Err(e) = ::nebula_validator::foundation::Validate::validate(
                            &::nebula_validator::validators::size_range::<#element_type>(#min_size, #max_size),
                            value.as_slice(),
                        ) {
                            errors.add(e.with_field(#field_key));
                        }
                    }
                }
            } else {
                quote! {
                    if let Err(e) = ::nebula_validator::foundation::Validate::validate(
                        &::nebula_validator::validators::size_range::<#element_type>(#min_size, #max_size),
                        input.#field_name.as_slice(),
                    ) {
                        errors.add(e.with_field(#field_key));
                    }
                }
            };

            if let Some(message) = &field_message {
                field_checks.push(quote! {
                    let before = errors.len();
                    #check
                    let after = errors.len();
                    if after > before {
                        if let Some(last) = errors.last_mut() {
                            last.message = ::std::borrow::Cow::Owned(#message.to_string());
                        }
                    }
                });
            } else {
                field_checks.push(check);
            }
        }

        // Format validator flags — each calls a zero-arg factory from nebula_validator::validators
        for (flag, expr) in built_in_string_validator_flags() {
            if validate_attrs.has_flag(flag) {
                if !field_is_string {
                    return Err(syn::Error::new_spanned(
                        &field.ty,
                        format!("`{flag}` requires `String` or `Option<String>` fields"),
                    ));
                }

                let check = generate_str_validator_check(field_name, &field_key, is_option, expr);
                if let Some(message) = &field_message {
                    field_checks.push(quote! {
                        let before = errors.len();
                        #check
                        let after = errors.len();
                        if after > before {
                            if let Some(last) = errors.last_mut() {
                                last.message = ::std::borrow::Cow::Owned(#message.to_string());
                            }
                        }
                    });
                } else {
                    field_checks.push(check);
                }
            }
        }

        for (key, factory) in built_in_string_validator_factories() {
            if let Some(arg) = validate_attrs.get_string(key) {
                if !field_is_string {
                    return Err(syn::Error::new_spanned(
                        &field.ty,
                        format!("`{key} = ...` requires `String` or `Option<String>` fields"),
                    ));
                }

                let validator_expr = match factory {
                    "contains" => quote!(::nebula_validator::validators::contains(#arg)),
                    "starts_with" => quote!(::nebula_validator::validators::starts_with(#arg)),
                    "ends_with" => quote!(::nebula_validator::validators::ends_with(#arg)),
                    _ => unreachable!("unsupported string validator factory"),
                };
                let check =
                    generate_str_validator_check(field_name, &field_key, is_option, validator_expr);
                if let Some(message) = &field_message {
                    field_checks.push(quote! {
                        let before = errors.len();
                        #check
                        let after = errors.len();
                        if after > before {
                            if let Some(last) = errors.last_mut() {
                                last.message = ::std::borrow::Cow::Owned(#message.to_string());
                            }
                        }
                    });
                } else {
                    field_checks.push(check);
                }
            }
        }

        if validate_attrs.has_flag("is_true") {
            if !field_is_bool {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "`is_true` requires `bool` or `Option<bool>` fields",
                ));
            }
            let check = if is_option {
                quote! {
                    if let Some(value) = input.#field_name.as_ref() {
                        if let Err(e) = ::nebula_validator::foundation::Validate::validate(&::nebula_validator::validators::is_true(), value) {
                            errors.add(e.with_field(#field_key));
                        }
                    }
                }
            } else {
                quote! {
                    if let Err(e) = ::nebula_validator::foundation::Validate::validate(&::nebula_validator::validators::is_true(), &input.#field_name) {
                        errors.add(e.with_field(#field_key));
                    }
                }
            };
            if let Some(message) = &field_message {
                field_checks.push(quote! {
                    let before = errors.len();
                    #check
                    let after = errors.len();
                    if after > before {
                        if let Some(last) = errors.last_mut() {
                            last.message = ::std::borrow::Cow::Owned(#message.to_string());
                        }
                    }
                });
            } else {
                field_checks.push(check);
            }
        }

        if validate_attrs.has_flag("is_false") {
            if !field_is_bool {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "`is_false` requires `bool` or `Option<bool>` fields",
                ));
            }
            let check = if is_option {
                quote! {
                    if let Some(value) = input.#field_name.as_ref() {
                        if let Err(e) = ::nebula_validator::foundation::Validate::validate(&::nebula_validator::validators::is_false(), value) {
                            errors.add(e.with_field(#field_key));
                        }
                    }
                }
            } else {
                quote! {
                    if let Err(e) = ::nebula_validator::foundation::Validate::validate(&::nebula_validator::validators::is_false(), &input.#field_name) {
                        errors.add(e.with_field(#field_key));
                    }
                }
            };
            if let Some(message) = &field_message {
                field_checks.push(quote! {
                    let before = errors.len();
                    #check
                    let after = errors.len();
                    if after > before {
                        if let Some(last) = errors.last_mut() {
                            last.message = ::std::borrow::Cow::Owned(#message.to_string());
                        }
                    }
                });
            } else {
                field_checks.push(check);
            }
        }

        // regex = "pattern" key-value attribute
        if let Some(pattern) = validate_attrs.get_string("regex") {
            if !field_is_string {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "`regex = ...` requires `String` or `Option<String>` fields",
                ));
            }

            let check = generate_regex_validator_check(field_name, &field_key, is_option, &pattern);
            if let Some(message) = &field_message {
                field_checks.push(quote! {
                    let before = errors.len();
                    #check
                    let after = errors.len();
                    if after > before {
                        if let Some(last) = errors.last_mut() {
                            last.message = ::std::borrow::Cow::Owned(#message.to_string());
                        }
                    }
                });
            } else {
                field_checks.push(check);
            }
        }

        if validate_attrs.has_flag("nested") {
            if is_option {
                if let Some(message) = &field_message {
                    field_checks.push(quote! {
                        if let Some(value) = input.#field_name.as_ref() {
                            if let Err(mut e) = ::nebula_validator::combinators::SelfValidating::check(value) {
                                e = e.with_field(#field_key);
                                e.message = ::std::borrow::Cow::Owned(#message.to_string());
                                errors.add(e);
                            }
                        }
                    });
                } else {
                    field_checks.push(quote! {
                        if let Some(value) = input.#field_name.as_ref() {
                            if let Err(e) = ::nebula_validator::combinators::SelfValidating::check(value) {
                                errors.add(e.with_field(#field_key));
                            }
                        }
                    });
                }
            } else {
                if let Some(message) = &field_message {
                    field_checks.push(quote! {
                        if let Err(mut e) = ::nebula_validator::combinators::SelfValidating::check(&input.#field_name) {
                            e = e.with_field(#field_key);
                            e.message = ::std::borrow::Cow::Owned(#message.to_string());
                            errors.add(e);
                        }
                    });
                } else {
                    field_checks.push(quote! {
                        if let Err(e) = ::nebula_validator::combinators::SelfValidating::check(&input.#field_name) {
                            errors.add(e.with_field(#field_key));
                        }
                    });
                }
            }
        }

        if let Some(custom_expr) = parse_custom_validator_expr(&validate_attrs)? {
            if is_option {
                if let Some(message) = &field_message {
                    field_checks.push(quote! {
                        if let Some(value) = input.#field_name.as_ref() {
                            if let Err(mut e) = (#custom_expr)(value) {
                                e = e.with_field(#field_key);
                                e.message = ::std::borrow::Cow::Owned(#message.to_string());
                                errors.add(e);
                            }
                        }
                    });
                } else {
                    field_checks.push(quote! {
                        if let Some(value) = input.#field_name.as_ref() {
                            if let Err(e) = (#custom_expr)(value) {
                                errors.add(e.with_field(#field_key));
                            }
                        }
                    });
                }
            } else {
                if let Some(message) = &field_message {
                    field_checks.push(quote! {
                        if let Err(mut e) = (#custom_expr)(&input.#field_name) {
                            e = e.with_field(#field_key);
                            e.message = ::std::borrow::Cow::Owned(#message.to_string());
                            errors.add(e);
                        }
                    });
                } else {
                    field_checks.push(quote! {
                        if let Err(e) = (#custom_expr)(&input.#field_name) {
                            errors.add(e.with_field(#field_key));
                        }
                    });
                }
            }
        }

        if let Some(each_attrs) = each_attrs {
            let element_source_type = option_inner_type(&field.ty).unwrap_or(&field.ty);
            let element_type = vec_inner_type(element_source_type).ok_or_else(|| {
                syn::Error::new_spanned(
                    &field.ty,
                    "`each(...)` is supported for `Vec<T>` and `Option<Vec<T>>` fields",
                )
            })?;

            let each_is_option = option_inner_type(&field.ty).is_some();
            let each_element_is_string = is_string_type(element_type);
            let mut each_checks = Vec::new();

            if let Some(min_len) = parse_usize(&each_attrs, "min_length")? {
                if let Some(message) = &field_message {
                    each_checks.push(quote! {
                        if value.len() < #min_len {
                            let mut err = ::nebula_validator::foundation::ValidationError::min_length(
                                each_field.clone(),
                                #min_len,
                                value.len(),
                            );
                            err.message = ::std::borrow::Cow::Owned(#message.to_string());
                            errors.add(err);
                        }
                    });
                } else {
                    each_checks.push(quote! {
                        if value.len() < #min_len {
                            errors.add(::nebula_validator::foundation::ValidationError::min_length(
                                each_field.clone(),
                                #min_len,
                                value.len(),
                            ));
                        }
                    });
                }
            }

            if let Some(max_len) = parse_usize(&each_attrs, "max_length")? {
                if let Some(message) = &field_message {
                    each_checks.push(quote! {
                        if value.len() > #max_len {
                            let mut err = ::nebula_validator::foundation::ValidationError::max_length(
                                each_field.clone(),
                                #max_len,
                                value.len(),
                            );
                            err.message = ::std::borrow::Cow::Owned(#message.to_string());
                            errors.add(err);
                        }
                    });
                } else {
                    each_checks.push(quote! {
                        if value.len() > #max_len {
                            errors.add(::nebula_validator::foundation::ValidationError::max_length(
                                each_field.clone(),
                                #max_len,
                                value.len(),
                            ));
                        }
                    });
                }
            }

            if let Some(exact_len) = parse_usize(&each_attrs, "exact_length")? {
                if let Some(message) = &field_message {
                    each_checks.push(quote! {
                        if value.len() != #exact_len {
                            let mut err = ::nebula_validator::foundation::ValidationError::exact_length(
                                each_field.clone(),
                                #exact_len,
                                value.len(),
                            );
                            err.message = ::std::borrow::Cow::Owned(#message.to_string());
                            errors.add(err);
                        }
                    });
                } else {
                    each_checks.push(quote! {
                        if value.len() != #exact_len {
                            errors.add(::nebula_validator::foundation::ValidationError::exact_length(
                                each_field.clone(),
                                #exact_len,
                                value.len(),
                            ));
                        }
                    });
                }
            }

            if let Some(min_value) = parse_number_lit(&each_attrs, "min")? {
                if let Some(message) = &field_message {
                    each_checks.push(quote! {
                        if value < &#min_value {
                            let mut err = ::nebula_validator::foundation::ValidationError::new(
                                "min",
                                format!("{} must be >= {}", each_field, #min_value),
                            )
                            .with_field(each_field.clone());
                            err.message = ::std::borrow::Cow::Owned(#message.to_string());
                            errors.add(err);
                        }
                    });
                } else {
                    each_checks.push(quote! {
                        if value < &#min_value {
                            errors.add(
                                ::nebula_validator::foundation::ValidationError::new(
                                    "min",
                                    format!("{} must be >= {}", each_field, #min_value),
                                )
                                .with_field(each_field.clone()),
                            );
                        }
                    });
                }
            }

            if let Some(max_value) = parse_number_lit(&each_attrs, "max")? {
                if let Some(message) = &field_message {
                    each_checks.push(quote! {
                        if value > &#max_value {
                            let mut err = ::nebula_validator::foundation::ValidationError::new(
                                "max",
                                format!("{} must be <= {}", each_field, #max_value),
                            )
                            .with_field(each_field.clone());
                            err.message = ::std::borrow::Cow::Owned(#message.to_string());
                            errors.add(err);
                        }
                    });
                } else {
                    each_checks.push(quote! {
                        if value > &#max_value {
                            errors.add(
                                ::nebula_validator::foundation::ValidationError::new(
                                    "max",
                                    format!("{} must be <= {}", each_field, #max_value),
                                )
                                .with_field(each_field.clone()),
                            );
                        }
                    });
                }
            }

            for (flag, expr) in built_in_string_validator_flags() {
                if each_attrs.has_flag(flag) {
                    if !each_element_is_string {
                        return Err(syn::Error::new_spanned(
                            &field.ty,
                            format!(
                                "`each({flag})` requires `Vec<String>` or `Option<Vec<String>>`"
                            ),
                        ));
                    }

                    if let Some(message) = &field_message {
                        each_checks.push(quote! {
                            if let Err(mut e) = ::nebula_validator::foundation::Validate::validate(&#expr, value.as_str()) {
                                e = e.with_field(each_field.clone());
                                e.message = ::std::borrow::Cow::Owned(#message.to_string());
                                errors.add(e);
                            }
                        });
                    } else {
                        each_checks.push(quote! {
                            if let Err(e) = ::nebula_validator::foundation::Validate::validate(&#expr, value.as_str()) {
                                errors.add(e.with_field(each_field.clone()));
                            }
                        });
                    }
                }
            }

            for (key, factory) in built_in_string_validator_factories() {
                if let Some(arg) = each_attrs.get_string(key) {
                    if !each_element_is_string {
                        return Err(syn::Error::new_spanned(
                            &field.ty,
                            format!(
                                "`each({key} = ...)` requires `Vec<String>` or `Option<Vec<String>>`"
                            ),
                        ));
                    }

                    let validator_expr = match factory {
                        "contains" => quote!(::nebula_validator::validators::contains(#arg)),
                        "starts_with" => {
                            quote!(::nebula_validator::validators::starts_with(#arg))
                        }
                        "ends_with" => quote!(::nebula_validator::validators::ends_with(#arg)),
                        _ => unreachable!("unsupported string validator factory"),
                    };

                    if let Some(message) = &field_message {
                        each_checks.push(quote! {
                            if let Err(mut e) = ::nebula_validator::foundation::Validate::validate(&#validator_expr, value.as_str()) {
                                e = e.with_field(each_field.clone());
                                e.message = ::std::borrow::Cow::Owned(#message.to_string());
                                errors.add(e);
                            }
                        });
                    } else {
                        each_checks.push(quote! {
                            if let Err(e) = ::nebula_validator::foundation::Validate::validate(&#validator_expr, value.as_str()) {
                                errors.add(e.with_field(each_field.clone()));
                            }
                        });
                    }
                }
            }

            if let Some(pattern) = each_attrs.get_string("regex") {
                if !each_element_is_string {
                    return Err(syn::Error::new_spanned(
                        &field.ty,
                        "`each(regex = ...)` requires `Vec<String>` or `Option<Vec<String>>`",
                    ));
                }

                if let Some(message) = &field_message {
                    each_checks.push(quote! {
                        match ::nebula_validator::validators::matches_regex(#pattern) {
                            Ok(v) => {
                                if let Err(mut e) = ::nebula_validator::foundation::Validate::validate(&v, value.as_str()) {
                                    e = e.with_field(each_field.clone());
                                    e.message = ::std::borrow::Cow::Owned(#message.to_string());
                                    errors.add(e);
                                }
                            }
                            Err(e) => {
                                let mut err = ::nebula_validator::foundation::ValidationError::new(
                                    "invalid_regex_pattern",
                                    format!("invalid regex pattern `{}`: {}", #pattern, e),
                                )
                                .with_field(each_field.clone());
                                err.message = ::std::borrow::Cow::Owned(#message.to_string());
                                errors.add(err);
                            }
                        }
                    });
                } else {
                    each_checks.push(quote! {
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
                                    )
                                    .with_field(each_field.clone()),
                                );
                            }
                        }
                    });
                }
            }

            if each_attrs.has_flag("nested") {
                if let Some(message) = &field_message {
                    each_checks.push(quote! {
                        if let Err(mut e) = ::nebula_validator::combinators::SelfValidating::check(value) {
                            e = e.with_field(each_field.clone());
                            e.message = ::std::borrow::Cow::Owned(#message.to_string());
                            errors.add(e);
                        }
                    });
                } else {
                    each_checks.push(quote! {
                        if let Err(e) = ::nebula_validator::combinators::SelfValidating::check(value) {
                            errors.add(e.with_field(each_field.clone()));
                        }
                    });
                }
            }

            if let Some(custom_expr) = parse_custom_validator_expr(&each_attrs)? {
                if let Some(message) = &field_message {
                    each_checks.push(quote! {
                        if let Err(mut e) = (#custom_expr)(value) {
                            e = e.with_field(each_field.clone());
                            e.message = ::std::borrow::Cow::Owned(#message.to_string());
                            errors.add(e);
                        }
                    });
                } else {
                    each_checks.push(quote! {
                        if let Err(e) = (#custom_expr)(value) {
                            errors.add(e.with_field(each_field.clone()));
                        }
                    });
                }
            }

            let each_loop = quote! {
                for (index, value) in collection.iter().enumerate() {
                    let each_field = format!("{}[{}]", #field_key, index);
                    #(#each_checks)*
                }
            };

            if each_is_option {
                checks.push(quote! {
                    if let Some(collection) = input.#field_name.as_ref() {
                        #each_loop
                    }
                });
            } else {
                checks.push(quote! {
                    let collection = &input.#field_name;
                    #each_loop
                });
            }
        }

        checks.extend(field_checks);
    }

    let expanded = quote! {
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
    };

    Ok(expanded.into())
}
