use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, parse_macro_input};

use crate::support::validation_codegen::{
    built_in_string_validator_flags, generate_cmp_check, generate_len_check,
    generate_regex_validator_check, generate_str_validator_check, is_option_type, parse_number_lit,
    parse_usize,
};
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
        for (flag, expr) in built_in_string_validator_flags() {
            if validate_attrs.has_flag(flag) {
                field_checks.push(generate_str_validator_check(
                    field_name, &field_key, is_option, expr,
                ));
            }
        }

        // regex = "pattern" key-value attribute
        if let Some(pattern) = validate_attrs.get_string("regex") {
            field_checks.push(generate_regex_validator_check(
                field_name, &field_key, is_option, &pattern,
            ));
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
    };

    Ok(expanded.into())
}
