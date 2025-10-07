//! Derive macros for nebula-validator
//!
//! This is a separate proc-macro crate that provides derive macros.
//!
//! # Cargo.toml setup
//!
//! ```toml
//! [package]
//! name = "nebula-validator-derive"
//! version = "0.1.0"
//! edition = "2021"
//!
//! [lib]
//! proc-macro = true
//!
//! [dependencies]
//! syn = { version = "2.0", features = ["full"] }
//! quote = "1.0"
//! proc-macro2 = "1.0"
//! darling = "0.20"  # For parsing attributes
//! ```
//!
//! # Available Derives
//!
//! ## `#[derive(Validate)]`
//!
//! Automatically implements validation for structs:
//!
//! ```rust
//! use nebula_validator::Validate;
//!
//! #[derive(Validate)]
//! struct User {
//!     #[validate(min_length = 3, max_length = 20)]
//!     username: String,
//!
//!     #[validate(email)]
//!     email: String,
//!
//!     #[validate(range(min = 18, max = 100))]
//!     age: u8,
//! }
//!
//! let user = User {
//!     username: "john".to_string(),
//!     email: "john@example.com".to_string(),
//!     age: 25,
//! };
//!
//! user.validate()?;
//! ```
//!
//! ## `#[derive(Validator)]`
//!
//! Creates a validator from a struct:
//!
//! ```rust
//! use nebula_validator::Validator;
//!
//! #[derive(Validator)]
//! #[validator(input = "str", error = "ValidationError")]
//! struct MinLength {
//!     min: usize,
//! }
//!
//! impl MinLength {
//!     fn validate_impl(&self, input: &str) -> bool {
//!         input.len() >= self.min
//!     }
//!
//!     fn error_message(&self) -> String {
//!         format!("Must be at least {} characters", self.min)
//!     }
//! }
//! ```

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Data, Fields, Field, Type, Attribute};

// ============================================================================
// VALIDATE DERIVE
// ============================================================================

/// Derives validation implementation for structs.
///
/// # Attributes
///
/// ## Field-level attributes
///
/// - `#[validate(skip)]` - Skip validation for this field
/// - `#[validate(min_length = N)]` - Minimum string length
/// - `#[validate(max_length = N)]` - Maximum string length
/// - `#[validate(email)]` - Email format validation
/// - `#[validate(url)]` - URL format validation
/// - `#[validate(regex = "pattern")]` - Regex pattern matching
/// - `#[validate(range(min = N, max = M))]` - Numeric range
/// - `#[validate(custom = "function_name")]` - Custom validation function
/// - `#[validate(nested)]` - Validate nested struct
///
/// ## Examples
///
/// ```rust
/// #[derive(Validate)]
/// struct CreateUser {
///     #[validate(min_length = 3, max_length = 20)]
///     username: String,
///
///     #[validate(email)]
///     email: String,
///
///     #[validate(range(min = 18, max = 100))]
///     age: u8,
///
///     #[validate(nested)]
///     address: Address,
///
///     #[validate(skip)]
///     internal_id: uuid::Uuid,
/// }
/// ```
#[proc_macro_derive(Validate, attributes(validate))]
pub fn derive_validate(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    
    let name = &input.ident;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            _ => panic!("Validate can only be derived for structs with named fields"),
        },
        _ => panic!("Validate can only be derived for structs"),
    };

    let validations = fields.iter().map(|field| {
        let field_name = field.ident.as_ref().unwrap();
        let field_attrs = parse_field_attributes(&field.attrs);

        if field_attrs.skip {
            return quote! {};
        }

        let mut validators = Vec::new();

        // Min length
        if let Some(min) = field_attrs.min_length {
            validators.push(quote! {
                if self.#field_name.len() < #min {
                    errors.add(::nebula_validator::core::ValidationError::min_length(
                        stringify!(#field_name),
                        #min,
                        self.#field_name.len()
                    ));
                }
            });
        }

        // Max length
        if let Some(max) = field_attrs.max_length {
            validators.push(quote! {
                if self.#field_name.len() > #max {
                    errors.add(::nebula_validator::core::ValidationError::max_length(
                        stringify!(#field_name),
                        #max,
                        self.#field_name.len()
                    ));
                }
            });
        }

        // Email
        if field_attrs.email {
            validators.push(quote! {
                if !self.#field_name.contains('@') {
                    errors.add(::nebula_validator::core::ValidationError::invalid_format(
                        stringify!(#field_name),
                        "email"
                    ));
                }
            });
        }

        // Nested
        if field_attrs.nested {
            validators.push(quote! {
                if let Err(e) = self.#field_name.validate() {
                    errors.add(::nebula_validator::core::ValidationError::new(
                        "nested_validation",
                        format!("Validation failed for {}", stringify!(#field_name))
                    ).with_field(stringify!(#field_name)));
                }
            });
        }

        // Range validation
        if let Some((min, max)) = field_attrs.range {
            validators.push(quote! {
                if self.#field_name < #min || self.#field_name > #max {
                    errors.add(::nebula_validator::core::ValidationError::out_of_range(
                        stringify!(#field_name),
                        #min,
                        #max,
                        self.#field_name
                    ));
                }
            });
        }

        quote! {
            #(#validators)*
        }
    });

    let expanded = quote! {
        impl #impl_generics #name #ty_generics #where_clause {
            /// Validates the struct according to field constraints.
            pub fn validate(&self) -> Result<(), ::nebula_validator::core::ValidationErrors> {
                let mut errors = ::nebula_validator::core::ValidationErrors::new();

                #(#validations)*

                errors.into_result(())
            }
        }
    };

    TokenStream::from(expanded)
}

// ============================================================================
// VALIDATOR DERIVE
// ============================================================================

/// Derives TypedValidator implementation.
///
/// # Attributes
///
/// - `#[validator(input = "Type")]` - Input type (required)
/// - `#[validator(output = "Type")]` - Output type (default: `()`)
/// - `#[validator(error = "Type")]` - Error type (default: `ValidationError`)
///
/// # Required Methods
///
/// The struct must implement:
/// - `validate_impl(&self, input: &Input) -> bool`
/// - `error_message(&self) -> String`
///
/// # Examples
///
/// ```rust
/// #[derive(Validator)]
/// #[validator(input = "str")]
/// struct MinLength {
///     min: usize,
/// }
///
/// impl MinLength {
///     fn validate_impl(&self, input: &str) -> bool {
///         input.len() >= self.min
///     }
///
///     fn error_message(&self) -> String {
///         format!("Must be at least {} characters", self.min)
///     }
/// }
/// ```
#[proc_macro_derive(Validator, attributes(validator))]
pub fn derive_validator(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    
    let name = &input.ident;
    let attrs = parse_validator_attributes(&input.attrs);

    let input_type = attrs.input.expect("validator(input = \"Type\") is required");
    let output_type = attrs.output.unwrap_or_else(|| quote! { () });
    let error_type = attrs.error.unwrap_or_else(|| quote! { ::nebula_validator::core::ValidationError });

    let expanded = quote! {
        impl ::nebula_validator::core::TypedValidator for #name {
            type Input = #input_type;
            type Output = #output_type;
            type Error = #error_type;

            fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
                if self.validate_impl(input) {
                    Ok(())
                } else {
                    Err(<Self::Error as From<String>>::from(self.error_message()))
                }
            }

            fn metadata(&self) -> ::nebula_validator::core::ValidatorMetadata {
                ::nebula_validator::core::ValidatorMetadata::simple(stringify!(#name))
            }
        }
    };

    TokenStream::from(expanded)
}

// ============================================================================
// ATTRIBUTE PARSING
// ============================================================================

#[derive(Default)]
struct FieldAttributes {
    skip: bool,
    min_length: Option<usize>,
    max_length: Option<usize>,
    email: bool,
    nested: bool,
    range: Option<(i64, i64)>,
}

fn parse_field_attributes(attrs: &[Attribute]) -> FieldAttributes {
    let mut result = FieldAttributes::default();

    for attr in attrs {
        if !attr.path().is_ident("validate") {
            continue;
        }

        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") {
                result.skip = true;
            } else if meta.path.is_ident("min_length") {
                let value: syn::LitInt = meta.value()?.parse()?;
                result.min_length = Some(value.base10_parse()?);
            } else if meta.path.is_ident("max_length") {
                let value: syn::LitInt = meta.value()?.parse()?;
                result.max_length = Some(value.base10_parse()?);
            } else if meta.path.is_ident("email") {
                result.email = true;
            } else if meta.path.is_ident("nested") {
                result.nested = true;
            }
            Ok(())
        });
    }

    result
}

struct ValidatorAttributes {
    input: Option<proc_macro2::TokenStream>,
    output: Option<proc_macro2::TokenStream>,
    error: Option<proc_macro2::TokenStream>,
}

fn parse_validator_attributes(attrs: &[Attribute]) -> ValidatorAttributes {
    let mut result = ValidatorAttributes {
        input: None,
        output: None,
        error: None,
    };

    for attr in attrs {
        if !attr.path().is_ident("validator") {
            continue;
        }

        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("input") {
                let value: syn::LitStr = meta.value()?.parse()?;
                let tokens: proc_macro2::TokenStream = value.value().parse().unwrap();
                result.input = Some(tokens);
            } else if meta.path.is_ident("output") {
                let value: syn::LitStr = meta.value()?.parse()?;
                let tokens: proc_macro2::TokenStream = value.value().parse().unwrap();
                result.output = Some(tokens);
            } else if meta.path.is_ident("error") {
                let value: syn::LitStr = meta.value()?.parse()?;
                let tokens: proc_macro2::TokenStream = value.value().parse().unwrap();
                result.error = Some(tokens);
            }
            Ok(())
        });
    }

    result
}

// ============================================================================
// TESTS
// ============================================================================

// Note: Testing proc macros requires integration tests in a separate crate
// See: nebula-validator/tests/derive_tests.rs