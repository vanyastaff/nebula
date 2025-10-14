//! Code generation for Validator derive using Field/MultiField combinators
//!
//! This module generates validation code using the shared infrastructure:
//! - `shared::validation` - for struct validation
//! - `shared::codegen` - for accessor and impl block generation
//! - `shared::attrs` - for attribute parsing
//! - `shared::types` - for type detection
//!
//! Validator generation logic is split into the `validators` submodule
//! for better organization and maintainability.

use super::parse::ValidationAttrs;
use super::validators;
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
        let Some(field_name) = &field.ident else {
            return Err(syn::Error::new_spanned(
                field,
                "named fields must have idents",
            ));
        };
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
    use crate::utils::field_name_to_error_field;

    // Convert field name to human-readable format for better error messages
    // e.g., "user_name" becomes "user name" in validation errors
    let field_name_str = field_name_to_error_field(&field_name.to_string());

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
/// Delegates to helper functions in the `validators` module for each
/// category of validators (string, numeric, collection, etc.).
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

    // PRIORITY: If expr is specified, use it directly
    if let Some(expr_str) = &attrs.expr {
        let expr: syn::Expr = syn::parse_str(expr_str)?;
        return Ok(quote! { #expr });
    }

    // Early return for nested validation
    if attrs.nested {
        return Ok(quote! {
            ::nebula_validator::combinators::nested::NestedValidator::new(|v| v.validate())
        });
    }

    // Collect validators by category using dedicated helper functions
    let mut validators = Vec::new();

    validators::add_string_validators(&mut validators, attrs);
    validators::add_text_validators(&mut validators, attrs);
    validators::add_numeric_validators(&mut validators, attrs);
    validators::add_collection_validators(&mut validators, attrs);
    validators::add_logical_validators(&mut validators, attrs);
    validators::add_custom_validator(&mut validators, attrs)?;

    // Chain validators with .and()
    validators::chain_validators(validators)
}
// NOTE: Field accessor generation is now handled directly by codegen::generate_accessor()
// in generate_field_addition() above. No wrapper function needed!
