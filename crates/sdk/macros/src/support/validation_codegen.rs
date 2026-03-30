use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::Type;

use crate::support::{attrs, diag};

pub fn parse_usize(args: &attrs::AttrArgs, key: &str) -> syn::Result<Option<usize>> {
    let value = match args.get_value(key) {
        Some(value) => value,
        None => return Ok(None),
    };

    let parsed = match value {
        attrs::AttrValue::Lit(syn::Lit::Int(int)) => int
            .base10_parse::<usize>()
            .map_err(|_| diag::error_spanned(int, format!("`{key}` must be a positive integer")))?,
        _ => {
            return Err(diag::error_spanned(
                &value_token(value),
                format!("`{key}` must be an integer literal"),
            ));
        }
    };

    Ok(Some(parsed))
}

pub fn parse_number_lit(args: &attrs::AttrArgs, key: &str) -> syn::Result<Option<TokenStream2>> {
    let value = match args.get_value(key) {
        Some(value) => value,
        None => return Ok(None),
    };

    match value {
        attrs::AttrValue::Lit(syn::Lit::Int(int)) => Ok(Some(quote!(#int))),
        attrs::AttrValue::Lit(syn::Lit::Float(float)) => Ok(Some(quote!(#float))),
        _ => Err(diag::error_spanned(
            &value_token(value),
            format!("`{key}` must be an integer or float literal"),
        )),
    }
}

pub fn generate_len_check(
    field_name: &syn::Ident,
    field_key: &str,
    is_option: bool,
    bound: usize,
    is_min: bool,
) -> TokenStream2 {
    let error = if is_min {
        quote! {
            ::nebula_validator::foundation::ValidationError::min_length(
                #field_key,
                #bound,
                value.len(),
            )
        }
    } else {
        quote! {
            ::nebula_validator::foundation::ValidationError::max_length(
                #field_key,
                #bound,
                value.len(),
            )
        }
    };

    if is_option {
        if is_min {
            quote! {
                if let Some(value) = input.#field_name.as_ref() {
                    if value.len() < #bound {
                        errors.add(#error);
                    }
                }
            }
        } else {
            quote! {
                if let Some(value) = input.#field_name.as_ref() {
                    if value.len() > #bound {
                        errors.add(#error);
                    }
                }
            }
        }
    } else if is_min {
        quote! {
            let value = &input.#field_name;
            if value.len() < #bound {
                errors.add(#error);
            }
        }
    } else {
        quote! {
            let value = &input.#field_name;
            if value.len() > #bound {
                errors.add(#error);
            }
        }
    }
}

pub fn generate_exact_len_check(
    field_name: &syn::Ident,
    field_key: &str,
    is_option: bool,
    expected: usize,
) -> TokenStream2 {
    let error = quote! {
        ::nebula_validator::foundation::ValidationError::exact_length(
            #field_key,
            #expected,
            value.len(),
        )
    };

    if is_option {
        quote! {
            if let Some(value) = input.#field_name.as_ref() {
                if value.len() != #expected {
                    errors.add(#error);
                }
            }
        }
    } else {
        quote! {
            let value = &input.#field_name;
            if value.len() != #expected {
                errors.add(#error);
            }
        }
    }
}

pub fn generate_cmp_check(
    field_name: &syn::Ident,
    field_key: &str,
    is_option: bool,
    bound: TokenStream2,
    is_min: bool,
) -> TokenStream2 {
    let error = if is_min {
        quote! {
            ::nebula_validator::foundation::ValidationError::new(
                "min",
                format!("{} must be >= {}", #field_key, #bound),
            )
            .with_field(#field_key)
        }
    } else {
        quote! {
            ::nebula_validator::foundation::ValidationError::new(
                "max",
                format!("{} must be <= {}", #field_key, #bound),
            )
            .with_field(#field_key)
        }
    };

    if is_option {
        if is_min {
            quote! {
                if let Some(value) = input.#field_name.as_ref() {
                    if value < &#bound {
                        errors.add(#error);
                    }
                }
            }
        } else {
            quote! {
                if let Some(value) = input.#field_name.as_ref() {
                    if value > &#bound {
                        errors.add(#error);
                    }
                }
            }
        }
    } else if is_min {
        quote! {
            let value = &input.#field_name;
            if value < &#bound {
                errors.add(#error);
            }
        }
    } else {
        quote! {
            let value = &input.#field_name;
            if value > &#bound {
                errors.add(#error);
            }
        }
    }
}

pub fn is_option_type(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.last()
    {
        return segment.ident == "Option";
    }
    false
}

pub fn generate_str_validator_check(
    field_name: &syn::Ident,
    field_key: &str,
    is_option: bool,
    validator_expr: TokenStream2,
) -> TokenStream2 {
    if is_option {
        quote! {
            if let Some(ref value) = input.#field_name {
                if let Err(e) = ::nebula_validator::foundation::Validate::validate(&#validator_expr, value.as_str()) {
                    errors.add(e.with_field(#field_key));
                }
            }
        }
    } else {
        quote! {
            if let Err(e) = ::nebula_validator::foundation::Validate::validate(&#validator_expr, input.#field_name.as_str()) {
                errors.add(e.with_field(#field_key));
            }
        }
    }
}

pub fn generate_regex_validator_check(
    field_name: &syn::Ident,
    field_key: &str,
    is_option: bool,
    pattern: &str,
) -> TokenStream2 {
    if is_option {
        quote! {
            if let Some(ref value) = input.#field_name {
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
                            )
                            .with_field(#field_key),
                        );
                    }
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
                        )
                        .with_field(#field_key),
                    );
                }
            }
        }
    }
}

pub fn built_in_string_validator_flags() -> Vec<(&'static str, TokenStream2)> {
    vec![
        (
            "not_empty",
            quote!(::nebula_validator::validators::not_empty()),
        ),
        (
            "alphanumeric",
            quote!(::nebula_validator::validators::alphanumeric()),
        ),
        (
            "alphabetic",
            quote!(::nebula_validator::validators::alphabetic()),
        ),
        ("numeric", quote!(::nebula_validator::validators::numeric())),
        (
            "lowercase",
            quote!(::nebula_validator::validators::lowercase()),
        ),
        (
            "uppercase",
            quote!(::nebula_validator::validators::uppercase()),
        ),
        ("email", quote!(::nebula_validator::validators::email())),
        ("url", quote!(::nebula_validator::validators::url())),
        ("ipv4", quote!(::nebula_validator::validators::ipv4())),
        ("ipv6", quote!(::nebula_validator::validators::ipv6())),
        ("ip_addr", quote!(::nebula_validator::validators::ip_addr())),
        (
            "hostname",
            quote!(::nebula_validator::validators::hostname()),
        ),
        ("uuid", quote!(::nebula_validator::validators::uuid())),
        ("date", quote!(::nebula_validator::validators::date())),
        (
            "date_time",
            quote!(::nebula_validator::validators::date_time()),
        ),
        ("time", quote!(::nebula_validator::validators::time())),
    ]
}

pub fn built_in_string_validator_factories() -> Vec<(&'static str, &'static str)> {
    vec![
        ("contains", "contains"),
        ("starts_with", "starts_with"),
        ("ends_with", "ends_with"),
    ]
}

pub fn value_token(value: &attrs::AttrValue) -> TokenStream2 {
    match value {
        attrs::AttrValue::Ident(ident) => quote!(#ident),
        attrs::AttrValue::Lit(lit) => quote!(#lit),
        attrs::AttrValue::Tokens(tokens) => tokens.clone(),
    }
}
