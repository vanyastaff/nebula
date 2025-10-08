//! Code generation for Validator derive using Field/MultiField combinators

use super::parse::ValidationAttrs;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, Type};

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

    // Generate .add_field() calls for each field
    let mut field_additions = Vec::new();

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_type = &field.ty;
        let attrs = ValidationAttrs::from_attributes(&field.attrs)?;

        if attrs.skip {
            continue;
        }

        if !attrs.has_validators() {
            continue;
        }

        let addition = generate_field_addition(name, field_name, field_type, &attrs)?;
        field_additions.push(addition);
    }

    // Generate the impl block using MultiField
    let generated = quote! {
        impl #name {
            /// Validates all fields of this struct using Field/MultiField combinators.
            ///
            /// Returns `Ok(())` if all validations pass, or an error with details
            /// about validation failures.
            pub fn validate(&self) -> ::std::result::Result<(), ::nebula_validator::core::ValidationError> {
                use ::nebula_validator::combinators::field::MultiField;
                use ::nebula_validator::core::TypedValidator;

                let validator = MultiField::new()
                    #(#field_additions)*;

                validator.validate(self).map(|_| ())
            }
        }
    };

    Ok(generated)
}

/// Generate a .add_field() call for a single field.
fn generate_field_addition(
    struct_name: &Ident,
    field_name: &Ident,
    field_type: &Type,
    attrs: &ValidationAttrs,
) -> syn::Result<TokenStream> {
    let field_name_str = field_name.to_string();

    // Generate the validator chain
    let validator = generate_field_validator(attrs)?;

    // Generate the accessor (closure that extracts the field)
    let accessor = generate_field_accessor(struct_name, field_name, field_type)?;

    Ok(quote! {
        .add_field(
            #field_name_str,
            #validator,
            #accessor
        )
    })
}

/// Generate the validator expression for a field.
fn generate_field_validator(attrs: &ValidationAttrs) -> syn::Result<TokenStream> {
    // PRIORITY: If expr is specified, use it directly
    if let Some(expr_str) = &attrs.expr {
        let expr: syn::Expr = syn::parse_str(expr_str)?;
        return Ok(quote! { #expr });
    }

    let mut validators = Vec::new();

    // String validators
    if let Some(min) = attrs.min_length {
        validators.push(quote! {
            ::nebula_validator::validators::string::min_length(#min)
        });
    }

    if let Some(max) = attrs.max_length {
        validators.push(quote! {
            ::nebula_validator::validators::string::max_length(#max)
        });
    }

    if let Some(exact) = attrs.exact_length {
        validators.push(quote! {
            ::nebula_validator::validators::string::exact_length(#exact)
        });
    }

    if attrs.email {
        validators.push(quote! {
            ::nebula_validator::validators::string::email()
        });
    }

    if attrs.url {
        validators.push(quote! {
            ::nebula_validator::validators::string::url()
        });
    }

    if let Some(pattern) = &attrs.regex {
        validators.push(quote! {
            ::nebula_validator::validators::string::matches_regex(#pattern).expect("Invalid regex pattern")
        });
    }

    if attrs.alphanumeric {
        validators.push(quote! {
            ::nebula_validator::validators::string::alphanumeric()
        });
    }

    if let Some(substring) = &attrs.contains {
        validators.push(quote! {
            ::nebula_validator::validators::string::contains(#substring)
        });
    }

    if let Some(prefix) = &attrs.starts_with {
        validators.push(quote! {
            ::nebula_validator::validators::string::starts_with(#prefix)
        });
    }

    if let Some(suffix) = &attrs.ends_with {
        validators.push(quote! {
            ::nebula_validator::validators::string::ends_with(#suffix)
        });
    }

    // Text validators
    if attrs.uuid {
        validators.push(quote! {
            ::nebula_validator::validators::text::Uuid::new()
        });
    }

    if attrs.datetime {
        validators.push(quote! {
            ::nebula_validator::validators::text::DateTime::new()
        });
    }

    if attrs.json {
        validators.push(quote! {
            ::nebula_validator::validators::text::Json::new()
        });
    }

    if attrs.slug {
        validators.push(quote! {
            ::nebula_validator::validators::text::Slug::new()
        });
    }

    if attrs.hex {
        validators.push(quote! {
            ::nebula_validator::validators::text::Hex::new()
        });
    }

    if attrs.base64 {
        validators.push(quote! {
            ::nebula_validator::validators::text::Base64::new()
        });
    }

    // Numeric validators
    if let Some(min) = &attrs.min {
        validators.push(quote! {
            ::nebula_validator::validators::numeric::min(#min)
        });
    }

    if let Some(max) = &attrs.max {
        validators.push(quote! {
            ::nebula_validator::validators::numeric::max(#max)
        });
    }

    if let (Some(min), Some(max)) = (&attrs.range_min, &attrs.range_max) {
        validators.push(quote! {
            ::nebula_validator::validators::numeric::in_range(#min, #max)
        });
    }

    if attrs.positive {
        validators.push(quote! {
            ::nebula_validator::validators::numeric::positive()
        });
    }

    if attrs.negative {
        validators.push(quote! {
            ::nebula_validator::validators::numeric::negative()
        });
    }

    if attrs.even {
        validators.push(quote! {
            ::nebula_validator::validators::numeric::even()
        });
    }

    if attrs.odd {
        validators.push(quote! {
            ::nebula_validator::validators::numeric::odd()
        });
    }

    // Collection validators
    if let Some(min) = attrs.min_size {
        validators.push(quote! {
            ::nebula_validator::validators::collection::min_size(#min)
        });
    }

    if let Some(max) = attrs.max_size {
        validators.push(quote! {
            ::nebula_validator::validators::collection::max_size(#max)
        });
    }

    if attrs.unique {
        validators.push(quote! {
            ::nebula_validator::validators::collection::unique()
        });
    }

    if attrs.non_empty {
        validators.push(quote! {
            ::nebula_validator::validators::collection::non_empty()
        });
    }

    // Logical validators
    if attrs.required {
        validators.push(quote! {
            ::nebula_validator::validators::logical::required()
        });
    }

    // Nested validation - use NestedValidator
    if attrs.nested {
        return Ok(quote! {
            ::nebula_validator::combinators::nested::NestedValidator::new(|v| v.validate())
        });
    }

    // Custom validator
    if let Some(custom_fn) = &attrs.custom {
        let custom_ident = syn::parse_str::<Ident>(custom_fn)?;
        validators.push(quote! {
            #custom_ident
        });
    }

    // Chain validators with .and()
    if validators.is_empty() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "No validators specified for field",
        ));
    }

    if validators.len() == 1 {
        Ok(validators.into_iter().next().unwrap())
    } else {
        let mut iter = validators.into_iter();
        let first = iter.next().unwrap();
        let rest: Vec<_> = iter.collect();
        Ok(quote! {
            #first #(.and(#rest))*
        })
    }
}

/// Generate the field accessor closure.
///
/// For String fields: |obj| obj.field.as_str()
/// For other fields: |obj| &obj.field
fn generate_field_accessor(
    struct_name: &Ident,
    field_name: &Ident,
    field_type: &Type,
) -> syn::Result<TokenStream> {
    // Check if the field is a String
    let is_string = is_string_type(field_type);

    if is_string {
        Ok(quote! {
            |obj: &#struct_name| obj.#field_name.as_str()
        })
    } else {
        Ok(quote! {
            |obj: &#struct_name| &obj.#field_name
        })
    }
}

/// Check if a type is String.
fn is_string_type(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            return segment.ident == "String";
        }
    }
    false
}
