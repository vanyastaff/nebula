//! Code generation for Validator derive

use super::parse::ValidationAttrs;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident};

/// Generate the validator implementation for a struct.
pub fn generate_validator(input: &DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;

    let data = match &input.data {
        Data::Struct(data) => data,
        Data::Enum(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "Validator derive macro can only be applied to structs.\n\
                 \n\
                 For enums, consider using a custom validator or the 'expr' attribute:\n\
                 #[validate(expr = \"custom_enum_validator()\")]",
            ));
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "Validator derive macro can only be applied to structs.\n\
                 \n\
                 Unions are not supported for automatic validation.",
            ));
        }
    };

    let fields = match &data.fields {
        Fields::Named(fields) => &fields.named,
        Fields::Unnamed(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "Validator derive macro requires named fields.\n\
                 \n\
                 Example:\n\
                 struct MyStruct {\n\
                 \x20   #[validate(min_length = 3)]\n\
                 \x20   name: String,\n\
                 }\n\
                 \n\
                 Tuple structs are not supported.",
            ));
        }
        Fields::Unit => {
            return Err(syn::Error::new_spanned(
                input,
                "Validator derive macro cannot be applied to unit structs.\n\
                 \n\
                 Unit structs have no fields to validate.",
            ));
        }
    };

    // Generate validation code for each field
    let mut validations = Vec::new();

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let attrs = ValidationAttrs::from_attributes(&field.attrs)?;

        if attrs.skip {
            continue;
        }

        if !attrs.has_validators() {
            continue;
        }

        let field_validations = generate_field_validations(field_name, &attrs)?;
        validations.push(field_validations);
    }

    // Generate the impl block
    let generated = quote! {
        impl #name {
            /// Validates all fields of this struct.
            ///
            /// Returns `Ok(())` if all validations pass, or an error with details
            /// about the first validation failure.
            pub fn validate(&self) -> Result<(), nebula_validator::core::ValidationErrors> {
                let mut errors = nebula_validator::core::ValidationErrors::new();

                #(#validations)*

                if errors.has_errors() {
                    Err(errors)
                } else {
                    Ok(())
                }
            }
        }
    };

    Ok(generated)
}

/// Generate validation code for a single field.
fn generate_field_validations(
    field_name: &Ident,
    attrs: &ValidationAttrs,
) -> syn::Result<TokenStream> {
    let field_name_str = field_name.to_string();
    let mut validators = Vec::new();

    // ПРИОРИТЕТ: Universal expression (если указан, использовать только его)
    if let Some(expr_str) = &attrs.expr {
        let expr: syn::Expr = syn::parse_str(expr_str)?;
        validators.push(quote! {
            if let Err(e) = (#expr).validate(&self.#field_name) {
                errors.add(e.with_field(#field_name_str));
            }
        });

        // Если expr указан, игнорируем остальные валидаторы!
        // Это позволяет пользователю полностью контролировать валидацию
        return Ok(quote! {
            #(#validators)*
        });
    }

    // String validators
    if let Some(min) = attrs.min_length {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::string::min_length(#min)
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if let Some(max) = attrs.max_length {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::string::max_length(#max)
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if let Some(exact) = attrs.exact_length {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::string::exact_length(#exact)
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if attrs.email {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::string::email()
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if attrs.url {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::string::url()
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if let Some(pattern) = &attrs.regex {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::string::regex(#pattern)
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if attrs.alphanumeric {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::string::alphanumeric()
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if let Some(substring) = &attrs.contains {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::string::contains(#substring)
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if let Some(prefix) = &attrs.starts_with {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::string::starts_with(#prefix)
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if let Some(suffix) = &attrs.ends_with {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::string::ends_with(#suffix)
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    // Text validators
    if attrs.uuid {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::text::Uuid::new()
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if attrs.datetime {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::text::DateTime::new()
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if attrs.json {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::text::Json::new()
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if attrs.slug {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::text::Slug::new()
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if attrs.hex {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::text::Hex::new()
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if attrs.base64 {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::text::Base64::new()
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    // Numeric validators
    if let Some(min) = &attrs.min {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::numeric::min(#min)
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if let Some(max) = &attrs.max {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::numeric::max(#max)
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if let (Some(min), Some(max)) = (&attrs.range_min, &attrs.range_max) {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::numeric::in_range(#min, #max)
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if attrs.positive {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::numeric::positive()
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if attrs.negative {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::numeric::negative()
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if attrs.even {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::numeric::even()
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if attrs.odd {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::numeric::odd()
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    // Collection validators
    if let Some(min) = attrs.min_size {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::collection::min_size(#min)
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if let Some(max) = attrs.max_size {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::collection::max_size(#max)
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if attrs.unique {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::collection::unique()
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if attrs.non_empty {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::collection::non_empty()
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    // Logical validators
    if attrs.required {
        validators.push(quote! {
            if let Err(e) = nebula_validator::validators::logical::required()
                .validate(&self.#field_name)
            {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    if attrs.nested {
        validators.push(quote! {
            if let Err(e) = self.#field_name.validate() {
                errors.add_nested(#field_name_str, e);
            }
        });
    }

    if let Some(custom_fn) = &attrs.custom {
        let custom_ident = syn::parse_str::<Ident>(custom_fn)?;
        validators.push(quote! {
            if let Err(e) = #custom_ident(&self.#field_name) {
                errors.add(e.with_field(#field_name_str));
            }
        });
    }

    Ok(quote! {
        #(#validators)*
    })
}
