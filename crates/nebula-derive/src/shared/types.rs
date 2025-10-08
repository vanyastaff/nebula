//! Universal type detection and analysis for all derive macros
//!
//! This module provides type categorization that can be used by any derive macro
//! in nebula-derive (Validator, Action, Resource, Parameter, etc).

use syn::{GenericArgument, PathArguments, PathSegment, Type, TypePath};

// ============================================================================
// TYPE CATEGORY
// ============================================================================

/// Comprehensive type categorization for derive macros
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeCategory {
    // Primitives
    String,
    Str,
    Bool,
    
    // Numbers
    Integer(IntegerType),
    Float(FloatType),
    
    // Dates/Times
    DateTime(DateTimeType),
    
    // Collections
    Vec(Box<TypeCategory>),
    HashSet(Box<TypeCategory>),
    HashMap {
        key: Box<TypeCategory>,
        value: Box<TypeCategory>,
    },
    BTreeMap {
        key: Box<TypeCategory>,
        value: Box<TypeCategory>,
    },
    BTreeSet(Box<TypeCategory>),
    
    // Special wrappers
    Option(Box<TypeCategory>),
    Result {
        ok: Box<TypeCategory>,
        err: Box<TypeCategory>,
    },
    Arc(Box<TypeCategory>),
    Rc(Box<TypeCategory>),
    Box(Box<TypeCategory>),
    Cow(Box<TypeCategory>),
    
    // Nebula-specific types
    NebulaValue,       // nebula_value::Value
    Parameter,         // nebula_parameter::Parameter
    
    // Custom types
    CustomStruct(StructInfo),
    CustomEnum(String),
    
    // References
    Reference {
        mutable: bool,
        inner: Box<TypeCategory>,
    },
    
    // Unknown
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegerType {
    I8, I16, I32, I64, I128, ISize,
    U8, U16, U32, U64, U128, USize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatType {
    F32,
    F64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateTimeType {
    /// String with ISO 8601 validation
    IsoString,
    
    /// Native JavaScript-style date
    NativeDate,
    
    /// Chrono types (feature-gated)
    ChronoNaiveDate,
    ChronoNaiveDateTime,
    ChronoDateTime,
    
    /// Time crate types (feature-gated)
    TimeDate,
    TimeOffsetDateTime,
    TimePrimitiveDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructInfo {
    pub name: String,
    pub path: Option<String>,
    pub is_nebula_type: bool,
}

// ============================================================================
// TYPE DETECTION
// ============================================================================

/// Main type detection function
pub fn detect_type(ty: &Type) -> TypeCategory {
    match ty {
        Type::Path(type_path) => detect_from_path(type_path),
        Type::Reference(type_ref) => TypeCategory::Reference {
            mutable: type_ref.mutability.is_some(),
            inner: Box::new(detect_type(&type_ref.elem)),
        },
        _ => TypeCategory::Unknown,
    }
}

fn detect_from_path(type_path: &TypePath) -> TypeCategory {
    let segments = &type_path.path.segments;
    
    if segments.is_empty() {
        return TypeCategory::Unknown;
    }
    
    // Get the last segment (most specific type)
    let last_segment = segments.last().unwrap();
    let type_name = last_segment.ident.to_string();
    
    // Check for primitive types first
    match type_name.as_str() {
        // Strings
        "String" => return TypeCategory::String,
        "str" => return TypeCategory::Str,
        
        // Bool
        "bool" => return TypeCategory::Bool,
        
        // Integers
        "i8" => return TypeCategory::Integer(IntegerType::I8),
        "i16" => return TypeCategory::Integer(IntegerType::I16),
        "i32" => return TypeCategory::Integer(IntegerType::I32),
        "i64" => return TypeCategory::Integer(IntegerType::I64),
        "i128" => return TypeCategory::Integer(IntegerType::I128),
        "isize" => return TypeCategory::Integer(IntegerType::ISize),
        "u8" => return TypeCategory::Integer(IntegerType::U8),
        "u16" => return TypeCategory::Integer(IntegerType::U16),
        "u32" => return TypeCategory::Integer(IntegerType::U32),
        "u64" => return TypeCategory::Integer(IntegerType::U64),
        "u128" => return TypeCategory::Integer(IntegerType::U128),
        "usize" => return TypeCategory::Integer(IntegerType::USize),
        
        // Floats
        "f32" => return TypeCategory::Float(FloatType::F32),
        "f64" => return TypeCategory::Float(FloatType::F64),
        
        _ => {}
    }
    
    // Check for date/time types with path context
    if type_name == "NativeDate" {
        return TypeCategory::DateTime(DateTimeType::NativeDate);
    }
    
    if has_module_in_path(segments, "chrono") {
        match type_name.as_str() {
            "NaiveDate" => return TypeCategory::DateTime(DateTimeType::ChronoNaiveDate),
            "NaiveDateTime" => return TypeCategory::DateTime(DateTimeType::ChronoNaiveDateTime),
            "DateTime" => return TypeCategory::DateTime(DateTimeType::ChronoDateTime),
            _ => {}
        }
    }
    
    if has_module_in_path(segments, "time") {
        match type_name.as_str() {
            "Date" => return TypeCategory::DateTime(DateTimeType::TimeDate),
            "OffsetDateTime" => return TypeCategory::DateTime(DateTimeType::TimeOffsetDateTime),
            "PrimitiveDateTime" => return TypeCategory::DateTime(DateTimeType::TimePrimitiveDateTime),
            _ => {}
        }
    }
    
    // Check for nebula types
    if has_module_in_path(segments, "nebula_value") || has_module_in_path(segments, "nebula-value") {
        if type_name == "Value" {
            return TypeCategory::NebulaValue;
        }
    }
    
    if has_module_in_path(segments, "nebula_parameter") || has_module_in_path(segments, "nebula-parameter") {
        if type_name == "Parameter" {
            return TypeCategory::Parameter;
        }
    }
    
    // Check for generic types
    match type_name.as_str() {
        "Vec" => {
            if let Some(inner_ty) = extract_first_generic(&last_segment) {
                return TypeCategory::Vec(Box::new(detect_type(&inner_ty)));
            }
        },
        
        "HashSet" => {
            if let Some(inner_ty) = extract_first_generic(&last_segment) {
                return TypeCategory::HashSet(Box::new(detect_type(&inner_ty)));
            }
        },
        
        "BTreeSet" => {
            if let Some(inner_ty) = extract_first_generic(&last_segment) {
                return TypeCategory::BTreeSet(Box::new(detect_type(&inner_ty)));
            }
        },
        
        "HashMap" => {
            if let Some((key_ty, value_ty)) = extract_two_generics(&last_segment) {
                return TypeCategory::HashMap {
                    key: Box::new(detect_type(&key_ty)),
                    value: Box::new(detect_type(&value_ty)),
                };
            }
        },
        
        "BTreeMap" => {
            if let Some((key_ty, value_ty)) = extract_two_generics(&last_segment) {
                return TypeCategory::BTreeMap {
                    key: Box::new(detect_type(&key_ty)),
                    value: Box::new(detect_type(&value_ty)),
                };
            }
        },
        
        "Option" => {
            if let Some(inner_ty) = extract_first_generic(&last_segment) {
                return TypeCategory::Option(Box::new(detect_type(&inner_ty)));
            }
        },
        
        "Result" => {
            if let Some((ok_ty, err_ty)) = extract_two_generics(&last_segment) {
                return TypeCategory::Result {
                    ok: Box::new(detect_type(&ok_ty)),
                    err: Box::new(detect_type(&err_ty)),
                };
            }
        },
        
        "Arc" => {
            if let Some(inner_ty) = extract_first_generic(&last_segment) {
                return TypeCategory::Arc(Box::new(detect_type(&inner_ty)));
            }
        },
        
        "Rc" => {
            if let Some(inner_ty) = extract_first_generic(&last_segment) {
                return TypeCategory::Rc(Box::new(detect_type(&inner_ty)));
            }
        },
        
        "Box" => {
            if let Some(inner_ty) = extract_first_generic(&last_segment) {
                return TypeCategory::Box(Box::new(detect_type(&inner_ty)));
            }
        },
        
        "Cow" => {
            if let Some(inner_ty) = extract_first_generic(&last_segment) {
                return TypeCategory::Cow(Box::new(detect_type(&inner_ty)));
            }
        },
        
        _ => {}
    }
    
    // Check if it's a custom struct (uppercase first letter)
    if type_name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
        let full_path = segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::");
        
        let is_nebula = segments.iter().any(|s| {
            let name = s.ident.to_string();
            name.starts_with("nebula") || name.starts_with("Nebula")
        });
        
        return TypeCategory::CustomStruct(StructInfo {
            name: type_name.clone(),
            path: Some(full_path),
            is_nebula_type: is_nebula,
        });
    }
    
    TypeCategory::Unknown
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Check if a module name appears in the path
fn has_module_in_path(
    segments: &syn::punctuated::Punctuated<PathSegment, syn::token::PathSep>,
    module_name: &str,
) -> bool {
    segments.iter().any(|seg| {
        seg.ident == module_name || seg.ident.to_string().replace('_', "-") == module_name
    })
}

/// Extract the first generic type argument
fn extract_first_generic(segment: &PathSegment) -> Option<Type> {
    if let PathArguments::AngleBracketed(args) = &segment.arguments {
        if let Some(GenericArgument::Type(ty)) = args.args.first() {
            return Some(ty.clone());
        }
    }
    None
}

/// Extract two generic type arguments (for HashMap, Result, etc.)
fn extract_two_generics(segment: &PathSegment) -> Option<(Type, Type)> {
    if let PathArguments::AngleBracketed(args) = &segment.arguments {
        let mut iter = args.args.iter();
        
        if let Some(GenericArgument::Type(first)) = iter.next() {
            if let Some(GenericArgument::Type(second)) = iter.next() {
                return Some((first.clone(), second.clone()));
            }
        }
    }
    None
}

// ============================================================================
// UTILITY METHODS
// ============================================================================

impl TypeCategory {
    /// Check if this type is a date/time type
    pub fn is_datetime(&self) -> bool {
        matches!(self, TypeCategory::DateTime(_))
    }
    
    /// Check if this type supports date validation
    pub fn supports_date_validation(&self) -> bool {
        match self {
            TypeCategory::DateTime(_) => true,
            TypeCategory::String => true, // Can validate ISO strings
            TypeCategory::Option(inner) => inner.supports_date_validation(),
            _ => false,
        }
    }
    
    /// Check if this is a collection type
    pub fn is_collection(&self) -> bool {
        matches!(
            self,
            TypeCategory::Vec(_)
                | TypeCategory::HashSet(_)
                | TypeCategory::BTreeSet(_)
                | TypeCategory::HashMap { .. }
                | TypeCategory::BTreeMap { .. }
        )
    }
    
    /// Check if this is a wrapper type (Option, Result, Arc, etc.)
    pub fn is_wrapper(&self) -> bool {
        matches!(
            self,
            TypeCategory::Option(_)
                | TypeCategory::Result { .. }
                | TypeCategory::Arc(_)
                | TypeCategory::Rc(_)
                | TypeCategory::Box(_)
        )
    }
    
    /// Get the inner type if this is a wrapper
    pub fn unwrap_wrapper(&self) -> Option<&TypeCategory> {
        match self {
            TypeCategory::Option(inner)
            | TypeCategory::Arc(inner)
            | TypeCategory::Rc(inner)
            | TypeCategory::Box(inner)
            | TypeCategory::Cow(inner) => Some(inner),
            TypeCategory::Reference { inner, .. } => Some(inner),
            _ => None,
        }
    }
    
    /// Check if this is a custom user-defined type
    pub fn is_custom(&self) -> bool {
        matches!(
            self,
            TypeCategory::CustomStruct(_) | TypeCategory::CustomEnum(_)
        )
    }
    
    /// Check if type needs special accessor (like String -> .as_str())
    pub fn needs_special_accessor(&self) -> bool {
        match self {
            TypeCategory::String => true,
            TypeCategory::Option(inner) => matches!(**inner, TypeCategory::String),
            _ => false,
        }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_detect_string() {
        let ty: Type = syn::parse_quote!(String);
        assert_eq!(detect_type(&ty), TypeCategory::String);
    }
    
    #[test]
    fn test_detect_str() {
        let ty: Type = syn::parse_quote!(str);
        assert_eq!(detect_type(&ty), TypeCategory::Str);
    }
    
    #[test]
    fn test_detect_integers() {
        let ty: Type = syn::parse_quote!(i32);
        assert_eq!(detect_type(&ty), TypeCategory::Integer(IntegerType::I32));
        
        let ty: Type = syn::parse_quote!(u64);
        assert_eq!(detect_type(&ty), TypeCategory::Integer(IntegerType::U64));
    }
    
    #[test]
    fn test_detect_native_date() {
        let ty: Type = syn::parse_quote!(NativeDate);
        assert_eq!(
            detect_type(&ty),
            TypeCategory::DateTime(DateTimeType::NativeDate)
        );
    }
    
    #[test]
    fn test_detect_chrono_date() {
        let ty: Type = syn::parse_quote!(chrono::NaiveDate);
        assert_eq!(
            detect_type(&ty),
            TypeCategory::DateTime(DateTimeType::ChronoNaiveDate)
        );
    }
    
    #[test]
    fn test_detect_option() {
        let ty: Type = syn::parse_quote!(Option<String>);
        match detect_type(&ty) {
            TypeCategory::Option(inner) => {
                assert_eq!(*inner, TypeCategory::String);
            }
            _ => panic!("Expected Option<String>"),
        }
    }
    
    #[test]
    fn test_detect_vec() {
        let ty: Type = syn::parse_quote!(Vec<i32>);
        match detect_type(&ty) {
            TypeCategory::Vec(inner) => {
                assert_eq!(*inner, TypeCategory::Integer(IntegerType::I32));
            }
            _ => panic!("Expected Vec<i32>"),
        }
    }
    
    #[test]
    fn test_detect_hashmap() {
        let ty: Type = syn::parse_quote!(HashMap<String, i32>);
        match detect_type(&ty) {
            TypeCategory::HashMap { key, value } => {
                assert_eq!(*key, TypeCategory::String);
                assert_eq!(*value, TypeCategory::Integer(IntegerType::I32));
            }
            _ => panic!("Expected HashMap<String, i32>"),
        }
    }
    
    #[test]
    fn test_detect_custom_struct() {
        let ty: Type = syn::parse_quote!(Address);
        match detect_type(&ty) {
            TypeCategory::CustomStruct(info) => {
                assert_eq!(info.name, "Address");
            }
            _ => panic!("Expected custom struct"),
        }
    }
    
    #[test]
    fn test_supports_date_validation() {
        let native_date: Type = syn::parse_quote!(NativeDate);
        assert!(detect_type(&native_date).supports_date_validation());
        
        let string: Type = syn::parse_quote!(String);
        assert!(detect_type(&string).supports_date_validation());
        
        let integer: Type = syn::parse_quote!(i32);
        assert!(!detect_type(&integer).supports_date_validation());
    }
    
    #[test]
    fn test_is_collection() {
        let vec: Type = syn::parse_quote!(Vec<String>);
        assert!(detect_type(&vec).is_collection());
        
        let string: Type = syn::parse_quote!(String);
        assert!(!detect_type(&string).is_collection());
    }
}