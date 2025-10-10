//! Code generation for Validator derive using Field/MultiField combinators
//!
//! This module generates validation code using the shared infrastructure:
//! - `shared::validation` - for struct validation
//! - `shared::codegen` - for accessor and impl block generation
//! - `shared::attrs` - for attribute parsing
//! - `shared::types` - for type detection

use super::parse::ValidationAttrs;
use crate::shared::{attrs, codegen, types, validation};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Ident, Type};

/// Generate the validator implementation for a struct.
///
/// This function orchestrates the entire validation code generation process:
/// 1. Validates the input struct structure
/// 2. Processes each field to generate validators
/// 3. Builds the final impl block with `validate()` method
///
/// # Errors
///
/// Returns an error if:
/// - Input is not a struct with named fields
/// - Field attributes are invalid
/// - No validators are specified for any field
pub(super) fn generate_validator(input: &DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;

    // Use shared validation to check struct requirements
    let fields = validation::require_named_struct(input)?;

    // Generate .add_field() calls for each field
    let mut field_additions = Vec::new();

    for field in &fields.named {
        let field_name = field.ident.as_ref().expect("named fields must have idents");
        let field_type = &field.ty;

        // Use shared attrs to check if field should be skipped
        if attrs::should_skip(&field.attrs) {
            continue;
        }

        let attrs = ValidationAttrs::from_attributes(&field.attrs)?;

        // Skip fields without validators
        if !attrs.has_validators() {
            continue;
        }

        let addition = generate_field_addition(name, field_name, field_type, &attrs)?;
        field_additions.push(addition);
    }

    // Use shared codegen ImplBlockBuilder for cleaner impl generation
    let mut builder = codegen::ImplBlockBuilder::new(name.clone(), input.generics.clone());

    builder.add_method(quote! {
        /// Validates all fields of this struct using Field/MultiField combinators.
        ///
        /// Returns `Ok(())` if all validations pass, or an error with details
        /// about validation failures.
        ///
        /// # Errors
        ///
        /// Returns a `ValidationError` if any field validation fails.
        /// The error includes the field name and specific validation failure details.
        pub fn validate(&self) -> ::std::result::Result<(), ::nebula_validator::core::ValidationError> {
            use ::nebula_validator::combinators::field::MultiField;
            use ::nebula_validator::core::TypedValidator;

            let validator = MultiField::new()
                #(#field_additions)*;

            validator.validate(self).map(|_| ())
        }
    });

    Ok(builder.build())
}

/// Generate a .`add_field()` call for a single field.
///
/// This combines the validator generation and accessor generation into
/// a single `.add_field()` call for the `MultiField` combinator.
///
/// # Arguments
///
/// * `struct_name` - Name of the parent struct
/// * `field_name` - Name of the field being validated
/// * `field_type` - Type of the field
/// * `attrs` - Parsed validation attributes
///
/// # Returns
///
/// A `TokenStream` representing the `.add_field()` call
fn generate_field_addition(
    struct_name: &Ident,
    field_name: &Ident,
    field_type: &Type,
    attrs: &ValidationAttrs,
) -> syn::Result<TokenStream> {
    let field_name_str = field_name.to_string();

    // Generate the validator chain using attributes
    let validator = generate_field_validator(field_type, attrs)?;

    // Use shared codegen to generate type-aware accessor
    let accessor = codegen::generate_accessor(struct_name, field_name, field_type);

    Ok(quote! {
        .add_field(
            #field_name_str,
            #validator,
            #accessor
        )
    })
}

/// Generate the validator expression for a field.
///
/// Uses both the field type (for type-aware validation) and attributes
/// (for explicit validators) to generate the appropriate validator chain.
///
/// # Arguments
///
/// * `field_type` - The type of the field (for smart type detection)
/// * `attrs` - Validation attributes from #[validate(...)]
///
/// # Returns
///
/// A `TokenStream` representing the validator or chain of validators
fn generate_field_validator(
    field_type: &Type,
    attrs: &ValidationAttrs,
) -> syn::Result<TokenStream> {
    // Detect field type using shared infrastructure
    let _type_category = types::detect_type(field_type);

    // TODO: In future, use type_category for smarter default validators
    // For now, use explicit attributes as before
    // PRIORITY: If expr is specified, use it directly
    if let Some(expr_str) = &attrs.expr {
        let expr: syn::Expr = syn::parse_str(expr_str)?;
        return Ok(quote! { #expr });
    }

    let mut validators = Vec::new();

    // Helper macro to reduce boilerplate when adding validators
    macro_rules! add_validator {
        ($path:path, $value:expr) => {
            if let Some(val) = $value {
                validators.push(quote! { $path(#val) });
            }
        };
        ($path:path) => {
            validators.push(quote! { $path() });
        };
    }

    // String validators
    add_validator!(
        ::nebula_validator::validators::string::min_length,
        attrs.min_length
    );
    add_validator!(
        ::nebula_validator::validators::string::max_length,
        attrs.max_length
    );
    add_validator!(
        ::nebula_validator::validators::string::exact_length,
        attrs.exact_length
    );

    if attrs.email {
        add_validator!(::nebula_validator::validators::string::email);
    }

    if attrs.url {
        add_validator!(::nebula_validator::validators::string::url);
    }

    if let Some(pattern) = &attrs.regex {
        validators.push(quote! {
            ::nebula_validator::validators::string::matches_regex(#pattern)
                .expect("Invalid regex pattern")
        });
    }

    if attrs.alphanumeric {
        add_validator!(::nebula_validator::validators::string::alphanumeric);
    }

    add_validator!(
        ::nebula_validator::validators::string::contains,
        attrs.contains.as_ref()
    );
    add_validator!(
        ::nebula_validator::validators::string::starts_with,
        attrs.starts_with.as_ref()
    );
    add_validator!(
        ::nebula_validator::validators::string::ends_with,
        attrs.ends_with.as_ref()
    );

    // Text validators (builder pattern)
    // Note: These use the builder pattern Validator::new()
    if attrs.uuid {
        validators.push(quote! { ::nebula_validator::validators::text::Uuid::new() });
    }
    if attrs.datetime {
        validators.push(quote! { ::nebula_validator::validators::text::DateTime::new() });
    }
    if attrs.json {
        validators.push(quote! { ::nebula_validator::validators::text::Json::new() });
    }
    if attrs.slug {
        validators.push(quote! { ::nebula_validator::validators::text::Slug::new() });
    }
    if attrs.hex {
        validators.push(quote! { ::nebula_validator::validators::text::Hex::new() });
    }
    if attrs.base64 {
        validators.push(quote! { ::nebula_validator::validators::text::Base64::new() });
    }

    // Numeric validators
    add_validator!(
        ::nebula_validator::validators::numeric::min,
        attrs.min.as_ref()
    );
    add_validator!(
        ::nebula_validator::validators::numeric::max,
        attrs.max.as_ref()
    );

    if let (Some(min), Some(max)) = (&attrs.range_min, &attrs.range_max) {
        validators.push(quote! {
            ::nebula_validator::validators::numeric::in_range(#min, #max)
        });
    }

    if attrs.positive {
        add_validator!(::nebula_validator::validators::numeric::positive);
    }
    if attrs.negative {
        add_validator!(::nebula_validator::validators::numeric::negative);
    }
    if attrs.even {
        add_validator!(::nebula_validator::validators::numeric::even);
    }
    if attrs.odd {
        add_validator!(::nebula_validator::validators::numeric::odd);
    }

    // Collection validators
    add_validator!(
        ::nebula_validator::validators::collection::min_size,
        attrs.min_size
    );
    add_validator!(
        ::nebula_validator::validators::collection::max_size,
        attrs.max_size
    );

    if attrs.unique {
        add_validator!(::nebula_validator::validators::collection::unique);
    }
    if attrs.non_empty {
        add_validator!(::nebula_validator::validators::collection::non_empty);
    }

    // Logical validators
    if attrs.required {
        add_validator!(::nebula_validator::validators::logical::required);
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
        Ok(validators
            .into_iter()
            .next()
            .expect("validators vec has exactly 1 element"))
    } else {
        let mut iter = validators.into_iter();
        let first = iter.next().expect("validators vec has at least 2 elements");
        let rest: Vec<_> = iter.collect();
        Ok(quote! {
            #first #(.and(#rest))*
        })
    }
}

// NOTE: Field accessor generation is now handled directly by codegen::generate_accessor()
// in generate_field_addition() above. No wrapper function needed!
