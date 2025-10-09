//! Input validation for derive macros
//!
//! Provides common validation utilities for all derive macros.

use syn::{Data, DeriveInput, Fields, FieldsNamed};

// Re-export attribute utilities from attrs module
pub use super::attrs::{extract_doc_comments, has_attribute};

// ============================================================================
// STRUCT VALIDATION
// ============================================================================

/// Validate that input is a struct with named fields.
///
/// Returns the named fields if validation passes, otherwise returns an error
/// with a helpful message explaining what went wrong.
///
/// # Errors
///
/// Returns an error if:
/// - Input is an enum (not a struct)
/// - Input is a union (not supported)
/// - Struct has unnamed fields (tuple struct)
/// - Struct has no fields (unit struct)
///
/// # Examples
///
/// ```ignore
/// use nebula_derive::shared::validation::require_named_struct;
///
/// pub fn my_derive(input: DeriveInput) -> syn::Result<TokenStream> {
///     let fields = require_named_struct(&input)?;
///     // ... use fields
/// }
/// ```
pub fn require_named_struct(input: &DeriveInput) -> syn::Result<&FieldsNamed> {
    match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => Ok(fields),
            Fields::Unnamed(_) => Err(syn::Error::new_spanned(
                input,
                "This derive macro requires named fields.\n\
                 \n\
                 Example:\n\
                 struct MyStruct {\n\
                 \x20   field1: Type1,\n\
                 \x20   field2: Type2,\n\
                 }\n\
                 \n\
                 Tuple structs are not supported.",
            )),
            Fields::Unit => Err(syn::Error::new_spanned(
                input,
                "This derive macro cannot be applied to unit structs.\n\
                 \n\
                 Unit structs have no fields to process.",
            )),
        },
        Data::Enum(_) => Err(syn::Error::new_spanned(
            input,
            "This derive macro can only be applied to structs.\n\
             \n\
             For enums, consider using custom implementation or expression attributes.",
        )),
        Data::Union(_) => Err(syn::Error::new_spanned(
            input,
            "This derive macro cannot be applied to unions.\n\
             \n\
             Unions are not supported for derive macros.",
        )),
    }
}

/// Validate that struct has at least one field.
///
/// # Errors
///
/// Returns an error if the struct has no fields.
///
/// # Examples
///
/// ```ignore
/// let fields = require_named_struct(&input)?;
/// require_non_empty(fields)?;
/// ```
pub fn require_non_empty(fields: &FieldsNamed) -> syn::Result<()> {
    if fields.named.is_empty() {
        Err(syn::Error::new_spanned(
            fields,
            "Struct must have at least one field.\n\
             \n\
             Empty structs cannot be processed by this derive macro.",
        ))
    } else {
        Ok(())
    }
}

/// Check for conflicting attributes on a field or struct.
///
/// This function checks if any pair of conflicting attributes are used together
/// and returns an error if they are.
///
/// # Arguments
///
/// * `attrs` - The attributes to check
/// * `conflicts` - Pairs of attribute names that conflict with each other
///
/// # Errors
///
/// Returns an error if any conflicting attribute pair is found.
///
/// # Examples
///
/// ```ignore
/// use nebula_derive::shared::validation::check_conflicting_attrs;
///
/// let conflicts = &[
///     ("min", "exact"),
///     ("max", "exact"),
/// ];
///
/// check_conflicting_attrs(&field.attrs, conflicts)?;
/// ```
pub fn check_conflicting_attrs(
    attrs: &[syn::Attribute],
    conflicts: &[(&str, &str)],
) -> syn::Result<()> {
    for (attr1, attr2) in conflicts {
        let has_attr1 = has_attribute(attrs, attr1);
        let has_attr2 = has_attribute(attrs, attr2);

        if has_attr1 && has_attr2 {
            return Err(syn::Error::new_spanned(
                &attrs[0],
                format!(
                    "Attributes '{}' and '{}' cannot be used together.\n\
                     \n\
                     These attributes are mutually exclusive.",
                    attr1, attr2
                ),
            ));
        }
    }
    Ok(())
}

/// Check if struct or field is marked as deprecated.
///
/// # Examples
///
/// ```ignore
/// if is_deprecated(&input.attrs) {
///     eprintln!("Warning: This struct is deprecated");
/// }
/// ```
pub fn is_deprecated(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident("deprecated"))
}

/// Extract the `#[deprecated]` message if present.
///
/// # Examples
///
/// ```ignore
/// if let Some(msg) = get_deprecation_message(&field.attrs) {
///     eprintln!("Field is deprecated: {}", msg);
/// }
/// ```
pub fn get_deprecation_message(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("deprecated") {
            // Try to extract message from #[deprecated = "..."]
            if let syn::Meta::NameValue(meta_name_value) = &attr.meta {
                if let syn::Expr::Lit(expr_lit) = &meta_name_value.value {
                    if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                        return Some(lit_str.value());
                    }
                }
            }

            // Try to extract from #[deprecated(note = "...")]
            if let Ok(meta_list) = attr.meta.require_list() {
                let tokens = meta_list.tokens.to_string();
                // Simple extraction - would need proper parsing for production
                if let Some(start) = tokens.find("note") {
                    if let Some(quote_start) = tokens[start..].find('"') {
                        if let Some(quote_end) = tokens[start + quote_start + 1..].find('"') {
                            let msg = &tokens
                                [start + quote_start + 1..start + quote_start + 1 + quote_end];
                            return Some(msg.to_string());
                        }
                    }
                }
            }

            // Deprecated without message
            return Some("This item is deprecated".to_string());
        }
    }
    None
}

// ============================================================================
// COMMON ATTRIBUTE STRUCTURES
// ============================================================================

/// Common attributes that multiple derive macros might use.
#[derive(Debug, Clone, Default)]
pub struct CommonAttrs {
    /// Skip this field in code generation
    pub skip: bool,

    /// Custom name for this field
    pub rename: Option<String>,

    /// Documentation/description
    pub doc: Option<String>,

    /// Feature flag requirement
    pub feature: Option<String>,

    /// Deprecated marker
    pub deprecated: Option<String>,
}

impl CommonAttrs {
    /// Parse common attributes from a set of syn attributes.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let common = CommonAttrs::from_attributes(&field.attrs);
    /// if common.skip {
    ///     continue; // Skip this field
    /// }
    /// ```
    pub fn from_attributes(attrs: &[syn::Attribute]) -> Self {
        Self {
            skip: has_attribute(attrs, "skip"),
            rename: None, // Would need proper parsing
            doc: extract_doc_comments(attrs),
            feature: None, // Would need proper parsing
            deprecated: get_deprecation_message(attrs),
        }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;
    use syn::parse_quote;

    #[test]
    fn test_require_named_struct_valid() {
        let input: DeriveInput = parse_quote! {
            struct MyStruct {
                field1: String,
                field2: i32,
            }
        };

        let result = require_named_struct(&input);
        assert!(result.is_ok());
        let fields = result.unwrap();
        assert_eq!(fields.named.len(), 2);
    }

    #[test]
    fn test_require_named_struct_tuple() {
        let input: DeriveInput = parse_quote! {
            struct MyStruct(String, i32);
        };

        let result = require_named_struct(&input);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("named fields"));
    }

    #[test]
    fn test_require_named_struct_unit() {
        let input: DeriveInput = parse_quote! {
            struct MyStruct;
        };

        let result = require_named_struct(&input);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unit struct"));
    }

    #[test]
    fn test_require_named_struct_enum() {
        let input: DeriveInput = parse_quote! {
            enum MyEnum {
                Variant1,
                Variant2,
            }
        };

        let result = require_named_struct(&input);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("struct"));
    }

    #[test]
    fn test_require_non_empty_valid() {
        let fields: FieldsNamed = parse_quote! {
            { field1: String }
        };

        assert!(require_non_empty(&fields).is_ok());
    }

    #[test]
    fn test_require_non_empty_invalid() {
        let fields: FieldsNamed = parse_quote! {
            {}
        };

        let result = require_non_empty(&fields);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_doc_comments() {
        let input: DeriveInput = parse_quote! {
            /// This is a test struct
            /// with multiple lines
            struct MyStruct {
                field: String,
            }
        };

        let doc = extract_doc_comments(&input.attrs);
        assert!(doc.is_some());
        let doc_text = doc.unwrap();
        assert!(doc_text.contains("test struct"));
        assert!(doc_text.contains("multiple lines"));
    }

    #[test]
    fn test_is_deprecated() {
        let input: DeriveInput = parse_quote! {
            #[deprecated]
            struct MyStruct {
                field: String,
            }
        };

        assert!(is_deprecated(&input.attrs));
    }
}
