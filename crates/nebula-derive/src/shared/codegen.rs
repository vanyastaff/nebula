//! Code generation utilities for derive macros
//!
//! Provides common code generation helpers that can be used by any derive macro.

use crate::shared::types::{TypeCategory, detect_type};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Ident, Type};

// ============================================================================
// FIELD ACCESSORS
// ============================================================================

/// Generate accessor closure for a field based on its type.
///
/// This function intelligently generates the appropriate accessor based on
/// the field's type category:
/// - `String` → `|obj| obj.field.as_str()`
/// - `Option<String>` → `|obj| obj.field.as_deref()`
/// - `&str` → `|obj| obj.field`
/// - Other types → `|obj| &obj.field`
///
/// # Arguments
///
/// * `struct_name` - The name of the parent struct
/// * `field_name` - The name of the field
/// * `field_type` - The type of the field
///
/// # Examples
///
/// ```ignore
/// use nebula_derive::shared::codegen::generate_accessor;
///
/// let accessor = generate_accessor(
///     &parse_quote!(User),
///     &parse_quote!(name),
///     &parse_quote!(String),
/// );
/// // Generates: |obj: &User| obj.name.as_str()
/// ```
pub(crate) fn generate_accessor(
    struct_name: &Ident,
    field_name: &Ident,
    field_type: &Type,
) -> TokenStream {
    let type_category = detect_type(field_type);

    match type_category {
        // String needs .as_str()
        TypeCategory::String => quote! {
            |obj: &#struct_name| obj.#field_name.as_str()
        },

        // Option<String> needs .as_deref()
        TypeCategory::Option(inner) if matches!(*inner, TypeCategory::String) => quote! {
            |obj: &#struct_name| obj.#field_name.as_deref()
        },

        // &str and other references don't need additional &
        TypeCategory::Reference { .. } | TypeCategory::Str => quote! {
            |obj: &#struct_name| obj.#field_name
        },

        // Everything else gets a simple reference
        _ => quote! {
            |obj: &#struct_name| &obj.#field_name
        },
    }
}

/// Generate accessor for a specific type category.
///
/// This is a lower-level function that allows you to generate an accessor
/// when you already know the type category.
///
/// # Examples
///
/// ```ignore
/// let accessor = generate_accessor_for_category(
///     &parse_quote!(User),
///     &parse_quote!(age),
///     &TypeCategory::Integer(IntegerType::U32),
/// );
/// ```
pub(crate) fn generate_accessor_for_category(
    struct_name: &Ident,
    field_name: &Ident,
    type_category: &TypeCategory,
) -> TokenStream {
    match type_category {
        TypeCategory::String => quote! {
            |obj: &#struct_name| obj.#field_name.as_str()
        },

        TypeCategory::Option(inner) if matches!(**inner, TypeCategory::String) => quote! {
            |obj: &#struct_name| obj.#field_name.as_deref()
        },

        TypeCategory::Reference { .. } | TypeCategory::Str => quote! {
            |obj: &#struct_name| obj.#field_name
        },

        _ => quote! {
            |obj: &#struct_name| &obj.#field_name
        },
    }
}

// ============================================================================
// TYPE CONVERSIONS
// ============================================================================

/// Generate conversion code from one type to another.
///
/// Returns `None` if no conversion is needed or available.
///
/// # Examples
///
/// ```ignore
/// let conversion = generate_conversion(
///     &TypeCategory::String,
///     &TypeCategory::Str,
/// );
/// // Returns: Some(quote! { .as_str() })
/// ```
pub(crate) fn generate_conversion(from: &TypeCategory, to: &TypeCategory) -> Option<TokenStream> {
    match (from, to) {
        // String → &str
        (TypeCategory::String, TypeCategory::Str) => Some(quote! { .as_str() }),

        // &str → String
        (TypeCategory::Str, TypeCategory::String) => Some(quote! { .to_string() }),

        // Option<T> → Option<U>
        (TypeCategory::Option(inner_from), TypeCategory::Option(inner_to)) => {
            let inner_conv = generate_conversion(inner_from, inner_to)?;
            Some(quote! { .map(|x| x #inner_conv) })
        }

        // Same types don't need conversion
        (a, b) if a == b => None,

        // No conversion available
        _ => None,
    }
}

// ============================================================================
// CLONE/COPY HELPERS
// ============================================================================

/// Generate clone expression based on type category.
///
/// Returns `Some` for types that are Copy or Clone, `None` otherwise.
///
/// # Examples
///
/// ```ignore
/// let clone_expr = generate_clone(&TypeCategory::Integer(IntegerType::I32));
/// // Returns: Some(quote! { })  (Copy types don't need .clone())
/// ```
pub(crate) fn generate_clone(type_category: &TypeCategory) -> Option<CloneStrategy> {
    match type_category {
        // Primitives are Copy
        TypeCategory::Bool | TypeCategory::Integer(_) | TypeCategory::Float(_) => {
            Some(CloneStrategy::Copy)
        }

        // String, Arc/Rc, custom types, and collections need .clone()
        TypeCategory::String
        | TypeCategory::Arc(_)
        | TypeCategory::Rc(_)
        | TypeCategory::CustomStruct(_)
        | TypeCategory::Vec(_)
        | TypeCategory::HashSet(_)
        | TypeCategory::HashMap { .. }
        | TypeCategory::BTreeMap { .. }
        | TypeCategory::BTreeSet(_) => Some(CloneStrategy::Clone),

        // References are Copy
        TypeCategory::Reference { .. } => Some(CloneStrategy::Copy),

        // Option/Result clone if inner clones
        TypeCategory::Option(inner) | TypeCategory::Box(inner) => generate_clone(inner),

        _ => None,
    }
}

/// Strategy for cloning a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloneStrategy {
    /// Type is Copy, no .`clone()` needed
    Copy,
    /// Type needs .`clone()`
    Clone,
}

impl CloneStrategy {
    /// Generate the appropriate clone expression.
    pub(crate) fn to_tokens(self) -> TokenStream {
        match self {
            CloneStrategy::Copy => quote! {},
            CloneStrategy::Clone => quote! { .clone() },
        }
    }
}

// ============================================================================
// IMPL BLOCK BUILDER
// ============================================================================

/// Builder for generating impl blocks with proper generic handling.
///
/// This builder simplifies generating impl blocks by handling generics,
/// where clauses, and method generation in a consistent way.
///
/// # Examples
///
/// ```ignore
/// let mut builder = ImplBlockBuilder::new(
///     parse_quote!(MyStruct),
///     input.generics.clone(),
/// );
///
/// builder.add_method(quote! {
///     pub fn validate(&self) -> Result<(), Error> {
///         // ...
///     }
/// });
///
/// let impl_block = builder.build();
/// ```
pub(crate) struct ImplBlockBuilder {
    struct_name: Ident,
    generics: syn::Generics,
    methods: Vec<TokenStream>,
    trait_path: Option<syn::Path>,
}

impl ImplBlockBuilder {
    /// Create a new impl block builder.
    pub(crate) fn new(struct_name: Ident, generics: syn::Generics) -> Self {
        Self {
            struct_name,
            generics,
            methods: Vec::new(),
            trait_path: None,
        }
    }

    /// Create a new impl block builder for a trait.
    pub(crate) fn new_trait(
        struct_name: Ident,
        generics: syn::Generics,
        trait_path: syn::Path,
    ) -> Self {
        Self {
            struct_name,
            generics,
            methods: Vec::new(),
            trait_path: Some(trait_path),
        }
    }

    /// Add a method to the impl block.
    pub(crate) fn add_method(&mut self, method: TokenStream) -> &mut Self {
        self.methods.push(method);
        self
    }

    /// Build the final impl block.
    pub(crate) fn build(self) -> TokenStream {
        let struct_name = &self.struct_name;
        let (impl_generics, ty_generics, where_clause) = self.generics.split_for_impl();
        let methods = &self.methods;

        if let Some(trait_path) = &self.trait_path {
            quote! {
                impl #impl_generics #trait_path for #struct_name #ty_generics #where_clause {
                    #(#methods)*
                }
            }
        } else {
            quote! {
                impl #impl_generics #struct_name #ty_generics #where_clause {
                    #(#methods)*
                }
            }
        }
    }
}

// ============================================================================
// DEFAULT VALUE GENERATION
// ============================================================================

/// Generate default value expression for a type.
///
/// # Examples
///
/// ```ignore
/// let default = generate_default(&TypeCategory::String);
/// // Returns: Some(quote! { String::new() })
/// ```
pub(crate) fn generate_default(type_category: &TypeCategory) -> Option<TokenStream> {
    match type_category {
        TypeCategory::String => Some(quote! { String::new() }),
        TypeCategory::Bool => Some(quote! { false }),
        TypeCategory::Integer(_) => Some(quote! { 0 }),
        TypeCategory::Float(_) => Some(quote! { 0.0 }),
        TypeCategory::Vec(_) => Some(quote! { Vec::new() }),
        TypeCategory::HashSet(_) => Some(quote! { std::collections::HashSet::new() }),
        TypeCategory::HashMap { .. } => Some(quote! { std::collections::HashMap::new() }),
        TypeCategory::BTreeMap { .. } => Some(quote! { std::collections::BTreeMap::new() }),
        TypeCategory::BTreeSet(_) => Some(quote! { std::collections::BTreeSet::new() }),
        TypeCategory::Option(_) => Some(quote! { None }),
        _ => None,
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_generate_accessor_string() {
        let accessor = generate_accessor(
            &parse_quote!(User),
            &parse_quote!(name),
            &parse_quote!(String),
        );

        let expected = quote! {
            |obj: &User| obj.name.as_str()
        };

        assert_eq!(accessor.to_string(), expected.to_string());
    }

    #[test]
    fn test_generate_accessor_integer() {
        let accessor =
            generate_accessor(&parse_quote!(User), &parse_quote!(age), &parse_quote!(u32));

        let expected = quote! {
            |obj: &User| &obj.age
        };

        assert_eq!(accessor.to_string(), expected.to_string());
    }

    #[test]
    fn test_generate_accessor_option_string() {
        let accessor = generate_accessor(
            &parse_quote!(User),
            &parse_quote!(nickname),
            &parse_quote!(Option<String>),
        );

        let expected = quote! {
            |obj: &User| obj.nickname.as_deref()
        };

        assert_eq!(accessor.to_string(), expected.to_string());
    }

    #[test]
    fn test_clone_strategy() {
        assert_eq!(
            generate_clone(&TypeCategory::Integer(
                crate::shared::types::IntegerType::I32
            )),
            Some(CloneStrategy::Copy)
        );

        assert_eq!(
            generate_clone(&TypeCategory::String),
            Some(CloneStrategy::Clone)
        );
    }

    #[test]
    fn test_impl_block_builder() {
        let builder = ImplBlockBuilder::new(parse_quote!(MyStruct), parse_quote! { <T> });

        let impl_block = builder.build();

        assert!(impl_block.to_string().contains("impl"));
        assert!(impl_block.to_string().contains("MyStruct"));
    }
}
