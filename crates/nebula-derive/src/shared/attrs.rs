//! Common attribute parsing utilities

use syn::Attribute;

// ============================================================================
// DOCUMENTATION
// ============================================================================

/// Extract documentation comments from attributes
///
/// Combines all `#[doc = "..."]` attributes into a single string.
///
/// # Examples
///
/// ```ignore
/// /// This is a doc comment
/// /// on multiple lines
/// struct MyStruct;
///
/// // Returns: "This is a doc comment\non multiple lines"
/// ```
pub fn extract_doc_comments(attrs: &[Attribute]) -> Option<String> {
    let mut docs = Vec::new();
    
    for attr in attrs {
        if attr.path().is_ident("doc") {
            if let syn::Meta::NameValue(meta) = &attr.meta {
                if let syn::Expr::Lit(expr_lit) = &meta.value {
                    if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                        docs.push(lit_str.value().trim().to_string());
                    }
                }
            }
        }
    }
    
    if docs.is_empty() {
        None
    } else {
        Some(docs.join("\n"))
    }
}

/// Check if attribute with given name exists
pub fn has_attribute(attrs: &[Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident(name))
}

/// Find first attribute with given name
pub fn find_attribute<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a Attribute> {
    attrs.iter().find(|attr| attr.path().is_ident(name))
}

/// Extract string value from attribute like #[name = "value"]
pub fn extract_string_attr(attrs: &[Attribute], name: &str) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident(name) {
            if let syn::Meta::NameValue(meta) = &attr.meta {
                if let syn::Expr::Lit(expr_lit) = &meta.value {
                    if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                        return Some(lit_str.value());
                    }
                }
            }
        }
    }
    None
}

// ============================================================================
// COMMON PATTERNS
// ============================================================================

/// Check if field should be skipped
///
/// Looks for #[skip] or #[serde(skip)] or similar patterns
pub fn should_skip(attrs: &[Attribute]) -> bool {
    has_attribute(attrs, "skip")
        || attrs.iter().any(|attr| {
            if attr.path().is_ident("serde") {
                // Check for #[serde(skip)]
                if let Ok(list) = attr.meta.require_list() {
                    return list.tokens.to_string().contains("skip");
                }
            }
            false
        })
}

/// Extract rename from attributes
///
/// Looks for #[rename = "new_name"] or #[serde(rename = "new_name")]
pub fn extract_rename(attrs: &[Attribute]) -> Option<String> {
    // Direct rename
    if let Some(name) = extract_string_attr(attrs, "rename") {
        return Some(name);
    }
    
    // Serde rename
    for attr in attrs {
        if attr.path().is_ident("serde") {
            if let Ok(list) = attr.meta.require_list() {
                let tokens = list.tokens.to_string();
                if tokens.contains("rename") {
                    // Simple parse: rename = "value"
                    if let Some(start) = tokens.find('"') {
                        if let Some(end) = tokens[start + 1..].find('"') {
                            return Some(tokens[start + 1..start + 1 + end].to_string());
                        }
                    }
                }
            }
        }
    }
    
    None
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_extract_doc_comments() {
        let attrs: Vec<Attribute> = syn::parse_quote! {
            /// First line
            /// Second line
        };
        
        let docs = extract_doc_comments(&attrs);
        assert_eq!(docs, Some("First line\nSecond line".to_string()));
    }
    
    #[test]
    fn test_has_attribute() {
        let attrs: Vec<Attribute> = syn::parse_quote! {
            #[skip]
            #[other]
        };
        
        assert!(has_attribute(&attrs, "skip"));
        assert!(has_attribute(&attrs, "other"));
        assert!(!has_attribute(&attrs, "missing"));
    }
    
    #[test]
    fn test_extract_string_attr() {
        let attrs: Vec<Attribute> = syn::parse_quote! {
            #[rename = "new_name"]
        };
        
        assert_eq!(
            extract_string_attr(&attrs, "rename"),
            Some("new_name".to_string())
        );
    }
    
    #[test]
    fn test_should_skip() {
        let attrs1: Vec<Attribute> = syn::parse_quote! { #[skip] };
        assert!(should_skip(&attrs1));
        
        let attrs2: Vec<Attribute> = syn::parse_quote! { #[other] };
        assert!(!should_skip(&attrs2));
    }
}