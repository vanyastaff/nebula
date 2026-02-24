use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Data, DeriveInput, Type, parse_macro_input};

use crate::support::{attrs, diag};

pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match expand(input) {
        Ok(ts) => ts,
        Err(e) => diag::to_compile_error(e),
    }
}

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

        let is_option = is_option_type(&field.ty);
        let mut field_checks = Vec::new();
        let field_key = field_name.to_string();

        if validate_attrs.has_flag("required") && is_option {
            field_checks.push(quote! {
                if input.#field_name.is_none() {
                    errors.add(
                        ::nebula_validator::foundation::ValidationError::required(#field_key)
                    );
                }
            });
        }

        if let Some(min_len) = parse_usize(&validate_attrs, "min_length")? {
            field_checks.push(generate_len_check(
                field_name, &field_key, is_option, min_len, true,
            ));
        }

        if let Some(max_len) = parse_usize(&validate_attrs, "max_length")? {
            field_checks.push(generate_len_check(
                field_name, &field_key, is_option, max_len, false,
            ));
        }

        if let Some(min_value) = parse_number_lit(&validate_attrs, "min")? {
            field_checks.push(generate_cmp_check(
                field_name, &field_key, is_option, min_value, true,
            ));
        }

        if let Some(max_value) = parse_number_lit(&validate_attrs, "max")? {
            field_checks.push(generate_cmp_check(
                field_name, &field_key, is_option, max_value, false,
            ));
        }

        // Format validator flags — each calls a zero-arg factory from nebula_validator::validators
        let str_validators: &[(&str, TokenStream2)] = &[
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
        ];
        for (flag, expr) in str_validators {
            if validate_attrs.has_flag(flag) {
                field_checks.push(generate_str_validator_check(
                    field_name,
                    &field_key,
                    is_option,
                    expr.clone(),
                ));
            }
        }

        // regex = "pattern" key-value attribute
        if let Some(pattern) = validate_attrs.get_string("regex") {
            let check = if is_option {
                quote! {
                    if let Some(ref value) = input.#field_name {
                        match ::nebula_validator::validators::matches_regex(#pattern) {
                            Ok(v) => {
                                if let Err(e) = ::nebula_validator::foundation::Validate::validate(&v, value.as_str()) {
                                    errors.add(e.with_field(#field_key));
                                }
                            }
                            Err(_) => panic!(
                                concat!("invalid regex pattern in #[validate(regex = ...)] on field `", #field_key, "`")
                            ),
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
                        Err(_) => panic!(
                            concat!("invalid regex pattern in #[validate(regex = ...)] on field `", #field_key, "`")
                        ),
                    }
                }
            };
            field_checks.push(check);
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

        impl #impl_generics ::nebula_validator::foundation::Validate for #struct_name #ty_generics #where_clause {
            type Input = Self;

            fn validate(
                &self,
                input: &Self::Input,
            ) -> ::std::result::Result<(), ::nebula_validator::foundation::ValidationError> {
                let _ = self;
                input
                    .validate_fields()
                    .map_err(|errors| errors.into_single_error(#root_message))
            }
        }
    };

    Ok(expanded.into())
}

fn parse_usize(args: &attrs::AttrArgs, key: &str) -> syn::Result<Option<usize>> {
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

fn parse_number_lit(args: &attrs::AttrArgs, key: &str) -> syn::Result<Option<TokenStream2>> {
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

fn value_token(value: &attrs::AttrValue) -> TokenStream2 {
    match value {
        attrs::AttrValue::Ident(ident) => quote!(#ident),
        attrs::AttrValue::Lit(lit) => quote!(#lit),
        attrs::AttrValue::Tokens(tokens) => tokens.clone(),
    }
}

fn generate_len_check(
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

fn generate_cmp_check(
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

fn is_option_type(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.last()
    {
        return segment.ident == "Option";
    }
    false
}

/// Generates a check that calls a zero-argument string validator factory.
fn generate_str_validator_check(
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
