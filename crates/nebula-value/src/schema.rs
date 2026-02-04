//! Value Schema for type introspection
//!
//! This module provides a schema system for **describing** the expected structure
//! of Values. Schemas are descriptive, not validating — actual validation
//! belongs in `nebula-validator`.
//!
//! # Design Philosophy (SOLID)
//!
//! - **Single Responsibility**: Schema describes structure, validator validates
//! - **Open/Closed**: Schema types can be extended without modification
//! - **Interface Segregation**: Schema is minimal, validation is separate
//!
//! # Use Cases
//!
//! - **Type documentation**: Describe the shape of expected data
//! - **Schema inference**: Automatically derive schemas from values
//! - **Integration**: Convert to/from JSON Schema, TypeScript types, etc.
//! - **Code generation**: Generate types from schemas
//!
//! # Examples
//!
//! ## Creating schemas
//!
//! ```
//! use nebula_value::schema::{ValueSchema, ObjectSchema};
//! use nebula_value::ValueKind;
//!
//! // Simple type schema
//! let string_schema = ValueSchema::kind(ValueKind::String);
//!
//! // Object with specific fields
//! let user_schema = ValueSchema::object()
//!     .field("name", ValueSchema::string())
//!     .field("age", ValueSchema::integer())
//!     .optional_field("email", ValueSchema::string())
//!     .build();
//! ```
//!
//! ## Inferring schemas
//!
//! ```
//! use nebula_value::Value;
//! use nebula_value::collections::Object;
//!
//! let obj = Object::from_iter(vec![
//!     ("name".to_string(), Value::text("Alice")),
//!     ("age".to_string(), Value::integer(30)),
//! ]);
//! let value = Value::Object(obj);
//!
//! let schema = value.infer_schema();
//! // schema describes: { name: String, age: Integer }
//! ```
//!
//! ## Basic type checking (for simple cases)
//!
//! ```
//! use nebula_value::{Value, ValueKind};
//! use nebula_value::schema::ValueSchema;
//!
//! let schema = ValueSchema::integer();
//!
//! // Simple type check - for full validation use nebula-validator
//! assert!(schema.accepts_kind(ValueKind::Integer));
//! assert!(!schema.accepts_kind(ValueKind::String));
//! ```

use std::collections::HashMap;
use std::fmt;

use crate::core::kind::ValueKind;
use crate::core::value::Value;

// ============================================================================
// VALUE SCHEMA
// ============================================================================

/// Describes the expected structure and type of a Value
///
/// This is a **descriptive** schema — it tells you what shape data should have.
/// For actual validation logic, use `nebula-validator` which can validate
/// values against these schemas with full error reporting, async support, etc.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ValueSchema {
    /// Matches any value
    #[default]
    Any,

    /// Matches a specific kind
    Kind(ValueKind),

    /// Matches null specifically
    Null,

    /// Boolean type
    Boolean,

    /// Integer with optional constraints
    Integer {
        /// Minimum value (inclusive)
        min: Option<i64>,
        /// Maximum value (inclusive)
        max: Option<i64>,
    },

    /// Float with optional constraints
    Float {
        /// Minimum value (inclusive)
        min: Option<f64>,
        /// Maximum value (inclusive)
        max: Option<f64>,
    },

    /// String with optional constraints
    String {
        /// Minimum length
        min_length: Option<usize>,
        /// Maximum length
        max_length: Option<usize>,
        /// Pattern to match (stored as string - validation done by nebula-validator)
        pattern: Option<String>,
    },

    /// Array with element schema
    Array {
        /// Schema for array elements
        items: Box<ValueSchema>,
        /// Minimum number of items
        min_items: Option<usize>,
        /// Maximum number of items
        max_items: Option<usize>,
    },

    /// Object with field schemas
    Object(ObjectSchema),

    /// Nullable wrapper - accepts null or the inner type
    Nullable(Box<ValueSchema>),

    /// Union type - one of several schemas
    OneOf(Vec<ValueSchema>),

    /// Intersection type - must satisfy all schemas
    AllOf(Vec<ValueSchema>),

    /// Constant value - must equal exactly
    Const(Value),
}

impl ValueSchema {
    // ==================== Constructors ====================

    /// Create an "any" schema that accepts everything
    pub fn any() -> Self {
        Self::Any
    }

    /// Create a schema for a specific kind
    pub fn kind(kind: ValueKind) -> Self {
        Self::Kind(kind)
    }

    /// Create a null schema
    pub fn null() -> Self {
        Self::Null
    }

    /// Create a boolean schema
    pub fn boolean() -> Self {
        Self::Boolean
    }

    /// Create an integer schema
    pub fn integer() -> Self {
        Self::Integer {
            min: None,
            max: None,
        }
    }

    /// Create an integer schema with range
    pub fn integer_range(min: Option<i64>, max: Option<i64>) -> Self {
        Self::Integer { min, max }
    }

    /// Create a float schema
    pub fn float() -> Self {
        Self::Float {
            min: None,
            max: None,
        }
    }

    /// Create a float schema with range
    pub fn float_range(min: Option<f64>, max: Option<f64>) -> Self {
        Self::Float { min, max }
    }

    /// Create a numeric schema (integer or float)
    pub fn numeric() -> Self {
        Self::OneOf(vec![Self::integer(), Self::float()])
    }

    /// Create a string schema
    pub fn string() -> Self {
        Self::String {
            min_length: None,
            max_length: None,
            pattern: None,
        }
    }

    /// Create a string schema with length constraints
    pub fn string_length(min: Option<usize>, max: Option<usize>) -> Self {
        Self::String {
            min_length: min,
            max_length: max,
            pattern: None,
        }
    }

    /// Create a string schema with pattern
    pub fn string_pattern(pattern: impl Into<String>) -> Self {
        Self::String {
            min_length: None,
            max_length: None,
            pattern: Some(pattern.into()),
        }
    }

    /// Create an array schema
    pub fn array(items: ValueSchema) -> Self {
        Self::Array {
            items: Box::new(items),
            min_items: None,
            max_items: None,
        }
    }

    /// Create an array schema with size constraints
    pub fn array_sized(items: ValueSchema, min: Option<usize>, max: Option<usize>) -> Self {
        Self::Array {
            items: Box::new(items),
            min_items: min,
            max_items: max,
        }
    }

    /// Start building an object schema
    pub fn object() -> ObjectSchemaBuilder {
        ObjectSchemaBuilder::new()
    }

    /// Create an object schema from ObjectSchema
    pub fn from_object_schema(schema: ObjectSchema) -> Self {
        Self::Object(schema)
    }

    /// Make this schema nullable
    pub fn nullable(self) -> Self {
        Self::Nullable(Box::new(self))
    }

    /// Create a union schema (accepts any)
    pub fn one_of(schemas: Vec<ValueSchema>) -> Self {
        Self::OneOf(schemas)
    }

    /// Create an intersection schema (must satisfy all)
    pub fn all_of(schemas: Vec<ValueSchema>) -> Self {
        Self::AllOf(schemas)
    }

    /// Create a constant schema (exact match)
    pub fn constant(value: Value) -> Self {
        Self::Const(value)
    }

    // ==================== Type Checking (Simple) ====================

    /// Check if this schema accepts a given ValueKind
    ///
    /// This is a **simple type check** only - it doesn't validate constraints.
    /// For full validation with constraints, use `nebula-validator`.
    pub fn accepts_kind(&self, kind: ValueKind) -> bool {
        match self {
            Self::Any => true,
            Self::Kind(k) => *k == kind,
            Self::Null => kind == ValueKind::Null,
            Self::Boolean => kind == ValueKind::Boolean,
            Self::Integer { .. } => kind == ValueKind::Integer,
            Self::Float { .. } => kind == ValueKind::Float || kind == ValueKind::Integer,
            Self::String { .. } => kind == ValueKind::String,
            Self::Array { .. } => kind == ValueKind::Array,
            Self::Object(_) => kind == ValueKind::Object,
            Self::Nullable(inner) => kind == ValueKind::Null || inner.accepts_kind(kind),
            Self::OneOf(schemas) => schemas.iter().any(|s| s.accepts_kind(kind)),
            Self::AllOf(schemas) => schemas.iter().all(|s| s.accepts_kind(kind)),
            Self::Const(v) => v.kind() == kind,
        }
    }

    /// Helper to collect unique kinds into a vector
    fn collect_unique_kinds(kinds: &mut Vec<ValueKind>, new_kinds: Vec<ValueKind>) {
        for k in new_kinds {
            if !kinds.contains(&k) {
                kinds.push(k);
            }
        }
    }

    /// Get the expected ValueKind(s) for this schema
    pub fn expected_kinds(&self) -> Vec<ValueKind> {
        match self {
            Self::Any => vec![
                ValueKind::Null,
                ValueKind::Boolean,
                ValueKind::Integer,
                ValueKind::Float,
                ValueKind::String,
                ValueKind::Array,
                ValueKind::Object,
            ],
            Self::Kind(k) => vec![*k],
            Self::Null => vec![ValueKind::Null],
            Self::Boolean => vec![ValueKind::Boolean],
            Self::Integer { .. } => vec![ValueKind::Integer],
            Self::Float { .. } => vec![ValueKind::Float, ValueKind::Integer],
            Self::String { .. } => vec![ValueKind::String],
            Self::Array { .. } => vec![ValueKind::Array],
            Self::Object(_) => vec![ValueKind::Object],
            Self::Nullable(inner) => {
                let mut kinds = inner.expected_kinds();
                if !kinds.contains(&ValueKind::Null) {
                    kinds.push(ValueKind::Null);
                }
                kinds
            }
            Self::OneOf(schemas) => {
                let mut kinds = Vec::new();
                for s in schemas {
                    Self::collect_unique_kinds(&mut kinds, s.expected_kinds());
                }
                kinds
            }
            Self::AllOf(schemas) => {
                if schemas.is_empty() {
                    return vec![];
                }
                let first_kinds = schemas[0].expected_kinds();
                first_kinds
                    .into_iter()
                    .filter(|k| schemas[1..].iter().all(|s| s.accepts_kind(*k)))
                    .collect()
            }
            Self::Const(v) => vec![v.kind()],
        }
    }

    // ==================== Schema Properties ====================

    /// Check if this schema is nullable
    pub fn is_nullable(&self) -> bool {
        matches!(self, Self::Nullable(_) | Self::Null | Self::Any)
    }

    /// Check if this schema describes an object
    pub fn is_object_schema(&self) -> bool {
        matches!(self, Self::Object(_))
    }

    /// Check if this schema describes an array
    pub fn is_array_schema(&self) -> bool {
        matches!(self, Self::Array { .. })
    }

    /// Get the inner schema if nullable
    pub fn unwrap_nullable(&self) -> &Self {
        match self {
            Self::Nullable(inner) => inner,
            other => other,
        }
    }

    /// Get object schema if this is an object
    pub fn as_object_schema(&self) -> Option<&ObjectSchema> {
        match self {
            Self::Object(s) => Some(s),
            _ => None,
        }
    }

    /// Get array item schema if this is an array
    pub fn as_array_items(&self) -> Option<&ValueSchema> {
        match self {
            Self::Array { items, .. } => Some(items),
            _ => None,
        }
    }

    // ==================== Schema Combination ====================

    /// Merge with another schema (for intersection)
    pub fn and(self, other: ValueSchema) -> Self {
        match (self, other) {
            (Self::AllOf(mut schemas), Self::AllOf(more)) => {
                schemas.extend(more);
                Self::AllOf(schemas)
            }
            (Self::AllOf(mut schemas), other) => {
                schemas.push(other);
                Self::AllOf(schemas)
            }
            (this, Self::AllOf(mut schemas)) => {
                schemas.insert(0, this);
                Self::AllOf(schemas)
            }
            (this, other) => Self::AllOf(vec![this, other]),
        }
    }

    /// Combine with another schema (for union)
    pub fn or(self, other: ValueSchema) -> Self {
        match (self, other) {
            (Self::OneOf(mut schemas), Self::OneOf(more)) => {
                schemas.extend(more);
                Self::OneOf(schemas)
            }
            (Self::OneOf(mut schemas), other) => {
                schemas.push(other);
                Self::OneOf(schemas)
            }
            (this, Self::OneOf(mut schemas)) => {
                schemas.insert(0, this);
                Self::OneOf(schemas)
            }
            (this, other) => Self::OneOf(vec![this, other]),
        }
    }
}

impl fmt::Display for ValueSchema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any => write!(f, "any"),
            Self::Kind(k) => write!(f, "{:?}", k),
            Self::Null => write!(f, "null"),
            Self::Boolean => write!(f, "boolean"),
            Self::Integer {
                min: None,
                max: None,
            } => write!(f, "integer"),
            Self::Integer { min, max } => {
                write!(f, "integer(")?;
                if let Some(m) = min {
                    write!(f, "min={}", m)?;
                }
                if let Some(m) = max {
                    if min.is_some() {
                        write!(f, ", ")?;
                    }
                    write!(f, "max={}", m)?;
                }
                write!(f, ")")
            }
            Self::Float {
                min: None,
                max: None,
            } => write!(f, "float"),
            Self::Float { min, max } => {
                write!(f, "float(")?;
                if let Some(m) = min {
                    write!(f, "min={}", m)?;
                }
                if let Some(m) = max {
                    if min.is_some() {
                        write!(f, ", ")?;
                    }
                    write!(f, "max={}", m)?;
                }
                write!(f, ")")
            }
            Self::String {
                min_length: None,
                max_length: None,
                pattern: None,
            } => {
                write!(f, "string")
            }
            Self::String { .. } => write!(f, "string(...)"),
            Self::Array { items, .. } => write!(f, "array<{}>", items),
            Self::Object(obj) => write!(f, "object({})", obj),
            Self::Nullable(inner) => write!(f, "{}?", inner),
            Self::OneOf(schemas) => {
                write!(f, "oneOf[")?;
                for (i, s) in schemas.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    write!(f, "{}", s)?;
                }
                write!(f, "]")
            }
            Self::AllOf(schemas) => {
                write!(f, "allOf[")?;
                for (i, s) in schemas.iter().enumerate() {
                    if i > 0 {
                        write!(f, " & ")?;
                    }
                    write!(f, "{}", s)?;
                }
                write!(f, "]")
            }
            Self::Const(v) => write!(f, "const({:?})", v),
        }
    }
}

// ============================================================================
// OBJECT SCHEMA
// ============================================================================

/// Schema for object types
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ObjectSchema {
    /// Property schemas by name
    pub properties: HashMap<String, ValueSchema>,
    /// Required property names
    pub required: Vec<String>,
    /// Whether additional properties are allowed
    pub additional_properties: bool,
    /// Schema for additional properties (if allowed)
    pub additional_properties_schema: Option<Box<ValueSchema>>,
}

impl ObjectSchema {
    /// Create a new empty object schema
    pub fn new() -> Self {
        Self {
            properties: HashMap::new(),
            required: Vec::new(),
            additional_properties: true,
            additional_properties_schema: None,
        }
    }

    /// Check if a property is required
    pub fn is_required(&self, name: &str) -> bool {
        self.required.contains(&name.to_string())
    }

    /// Get property schema
    pub fn get_property(&self, name: &str) -> Option<&ValueSchema> {
        self.properties.get(name)
    }

    /// Get all property names
    pub fn property_names(&self) -> impl Iterator<Item = &String> {
        self.properties.keys()
    }
}

impl fmt::Display for ObjectSchema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{")?;
        for (i, (name, schema)) in self.properties.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            let required = if self.required.contains(name) {
                ""
            } else {
                "?"
            };
            write!(f, "{}{}: {}", name, required, schema)?;
        }
        write!(f, "}}")
    }
}

// ============================================================================
// OBJECT SCHEMA BUILDER
// ============================================================================

/// Builder for ObjectSchema
#[derive(Debug, Default)]
pub struct ObjectSchemaBuilder {
    properties: HashMap<String, ValueSchema>,
    required: Vec<String>,
    additional_properties: bool,
    additional_properties_schema: Option<Box<ValueSchema>>,
}

impl ObjectSchemaBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            properties: HashMap::new(),
            required: Vec::new(),
            additional_properties: true,
            additional_properties_schema: None,
        }
    }

    /// Add a required field
    pub fn field(mut self, name: impl Into<String>, schema: ValueSchema) -> Self {
        let name = name.into();
        self.required.push(name.clone());
        self.properties.insert(name, schema);
        self
    }

    /// Add an optional field
    pub fn optional_field(mut self, name: impl Into<String>, schema: ValueSchema) -> Self {
        self.properties.insert(name.into(), schema);
        self
    }

    /// Disallow additional properties
    pub fn strict(mut self) -> Self {
        self.additional_properties = false;
        self
    }

    /// Allow additional properties (default)
    pub fn extensible(mut self) -> Self {
        self.additional_properties = true;
        self
    }

    /// Allow additional properties with a specific schema
    pub fn additional_properties(mut self, schema: ValueSchema) -> Self {
        self.additional_properties = true;
        self.additional_properties_schema = Some(Box::new(schema));
        self
    }

    /// Build the schema
    pub fn build(self) -> ValueSchema {
        ValueSchema::Object(ObjectSchema {
            properties: self.properties,
            required: self.required,
            additional_properties: self.additional_properties,
            additional_properties_schema: self.additional_properties_schema,
        })
    }
}

// ============================================================================
// VALUE INFERENCE
// ============================================================================

impl Value {
    /// Infer a schema from this value
    ///
    /// Creates a schema that describes this value's structure.
    /// The inferred schema is descriptive — it doesn't include validation rules.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::Object;
    /// use nebula_value::schema::ValueSchema;
    ///
    /// let obj = Object::from_iter(vec![
    ///     ("name".to_string(), Value::text("Alice")),
    ///     ("age".to_string(), Value::integer(30)),
    /// ]);
    /// let value = Value::Object(obj);
    ///
    /// let schema = value.infer_schema();
    /// // schema describes: { name: string, age: integer }
    /// ```
    pub fn infer_schema(&self) -> ValueSchema {
        match self {
            Value::Null => ValueSchema::Null,
            Value::Boolean(_) => ValueSchema::boolean(),
            Value::Integer(_) => ValueSchema::integer(),
            Value::Float(_) => ValueSchema::float(),
            Value::Decimal(_) => ValueSchema::kind(ValueKind::Decimal),
            Value::Text(_) => ValueSchema::string(),
            Value::Bytes(_) => ValueSchema::kind(ValueKind::Bytes),
            Value::Array(arr) => {
                // Infer schema from first element, or Any if empty
                let item_schema = arr
                    .first()
                    .map(|v| v.infer_schema())
                    .unwrap_or(ValueSchema::Any);
                ValueSchema::array(item_schema)
            }
            Value::Object(obj) => {
                let mut builder = ObjectSchemaBuilder::new();
                for (key, value) in obj.entries() {
                    builder = builder.field(key.clone(), value.infer_schema());
                }
                builder.build()
            }
            #[cfg(feature = "temporal")]
            Value::Date(_) => ValueSchema::kind(ValueKind::Date),
            #[cfg(feature = "temporal")]
            Value::Time(_) => ValueSchema::kind(ValueKind::Time),
            #[cfg(feature = "temporal")]
            Value::DateTime(_) => ValueSchema::kind(ValueKind::DateTime),
            #[cfg(feature = "temporal")]
            Value::Duration(_) => ValueSchema::kind(ValueKind::Duration),
        }
    }

    /// Infer a more permissive schema from multiple values
    ///
    /// Useful for inferring a schema from a sample of data.
    pub fn infer_schema_from_samples(values: &[Value]) -> ValueSchema {
        if values.is_empty() {
            return ValueSchema::Any;
        }

        if values.len() == 1 {
            return values[0].infer_schema();
        }

        // Get all unique kinds
        let schemas: Vec<_> = values.iter().map(|v| v.infer_schema()).collect();

        // If all same, return that schema
        if schemas.windows(2).all(|w| w[0] == w[1]) {
            return schemas.into_iter().next().unwrap();
        }

        // Otherwise return union
        ValueSchema::OneOf(schemas)
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collections::{Array, Object};

    // ==================== Constructor Tests ====================

    #[test]
    fn test_any_schema() {
        let schema = ValueSchema::any();
        assert!(schema.accepts_kind(ValueKind::Integer));
        assert!(schema.accepts_kind(ValueKind::String));
        assert!(schema.accepts_kind(ValueKind::Null));
    }

    #[test]
    fn test_kind_schema() {
        let schema = ValueSchema::kind(ValueKind::Integer);
        assert!(schema.accepts_kind(ValueKind::Integer));
        assert!(!schema.accepts_kind(ValueKind::String));
    }

    #[test]
    fn test_null_schema() {
        let schema = ValueSchema::null();
        assert!(schema.accepts_kind(ValueKind::Null));
        assert!(!schema.accepts_kind(ValueKind::Integer));
    }

    #[test]
    fn test_boolean_schema() {
        let schema = ValueSchema::boolean();
        assert!(schema.accepts_kind(ValueKind::Boolean));
        assert!(!schema.accepts_kind(ValueKind::Integer));
    }

    #[test]
    fn test_integer_schema() {
        let schema = ValueSchema::integer();
        assert!(schema.accepts_kind(ValueKind::Integer));
        assert!(!schema.accepts_kind(ValueKind::Float));
    }

    #[test]
    fn test_float_schema() {
        let schema = ValueSchema::float();
        assert!(schema.accepts_kind(ValueKind::Float));
        // Float accepts integers (can be converted)
        assert!(schema.accepts_kind(ValueKind::Integer));
        assert!(!schema.accepts_kind(ValueKind::String));
    }

    #[test]
    fn test_string_schema() {
        let schema = ValueSchema::string();
        assert!(schema.accepts_kind(ValueKind::String));
        assert!(!schema.accepts_kind(ValueKind::Integer));
    }

    #[test]
    fn test_array_schema() {
        let schema = ValueSchema::array(ValueSchema::integer());
        assert!(schema.accepts_kind(ValueKind::Array));
        assert!(!schema.accepts_kind(ValueKind::Object));
    }

    #[test]
    fn test_object_schema() {
        let schema = ValueSchema::object()
            .field("name", ValueSchema::string())
            .build();
        assert!(schema.accepts_kind(ValueKind::Object));
        assert!(!schema.accepts_kind(ValueKind::Array));
    }

    #[test]
    fn test_nullable_schema() {
        let schema = ValueSchema::string().nullable();
        assert!(schema.accepts_kind(ValueKind::String));
        assert!(schema.accepts_kind(ValueKind::Null));
        assert!(!schema.accepts_kind(ValueKind::Integer));
    }

    #[test]
    fn test_one_of_schema() {
        let schema = ValueSchema::one_of(vec![ValueSchema::string(), ValueSchema::integer()]);
        assert!(schema.accepts_kind(ValueKind::String));
        assert!(schema.accepts_kind(ValueKind::Integer));
        assert!(!schema.accepts_kind(ValueKind::Boolean));
    }

    #[test]
    fn test_const_schema() {
        let schema = ValueSchema::constant(Value::text("expected"));
        assert!(schema.accepts_kind(ValueKind::String));
        assert!(!schema.accepts_kind(ValueKind::Integer));
    }

    // ==================== Builder Tests ====================

    #[test]
    fn test_object_builder() {
        let schema = ValueSchema::object()
            .field("name", ValueSchema::string())
            .optional_field("email", ValueSchema::string())
            .strict()
            .build();

        if let ValueSchema::Object(obj) = &schema {
            assert!(obj.is_required("name"));
            assert!(!obj.is_required("email"));
            assert!(!obj.additional_properties);
        } else {
            panic!("Expected Object schema");
        }
    }

    #[test]
    fn test_object_with_additional_properties() {
        let schema = ValueSchema::object()
            .field("id", ValueSchema::integer())
            .additional_properties(ValueSchema::string())
            .build();

        if let ValueSchema::Object(obj) = &schema {
            assert!(obj.additional_properties);
            assert!(obj.additional_properties_schema.is_some());
        } else {
            panic!("Expected Object schema");
        }
    }

    // ==================== Schema Properties Tests ====================

    #[test]
    fn test_is_nullable() {
        assert!(ValueSchema::null().is_nullable());
        assert!(ValueSchema::any().is_nullable());
        assert!(ValueSchema::string().nullable().is_nullable());
        assert!(!ValueSchema::string().is_nullable());
    }

    #[test]
    fn test_expected_kinds() {
        let schema = ValueSchema::integer();
        let kinds = schema.expected_kinds();
        assert_eq!(kinds, vec![ValueKind::Integer]);

        let schema = ValueSchema::string().nullable();
        let kinds = schema.expected_kinds();
        assert!(kinds.contains(&ValueKind::String));
        assert!(kinds.contains(&ValueKind::Null));
    }

    #[test]
    fn test_unwrap_nullable() {
        let inner = ValueSchema::string();
        let nullable = inner.clone().nullable();

        assert_eq!(nullable.unwrap_nullable(), &inner);
        assert_eq!(inner.unwrap_nullable(), &inner);
    }

    // ==================== Combination Tests ====================

    #[test]
    fn test_schema_and() {
        let s1 = ValueSchema::integer();
        let s2 = ValueSchema::integer_range(Some(0), None);
        let combined = s1.and(s2);

        assert!(matches!(combined, ValueSchema::AllOf(_)));
    }

    #[test]
    fn test_schema_or() {
        let s1 = ValueSchema::integer();
        let s2 = ValueSchema::string();
        let combined = s1.or(s2);

        assert!(matches!(combined, ValueSchema::OneOf(_)));
        assert!(combined.accepts_kind(ValueKind::Integer));
        assert!(combined.accepts_kind(ValueKind::String));
    }

    // ==================== Inference Tests ====================

    #[test]
    fn test_infer_scalar() {
        assert!(matches!(
            Value::integer(42).infer_schema(),
            ValueSchema::Integer { .. }
        ));
        assert!(matches!(
            Value::text("hi").infer_schema(),
            ValueSchema::String { .. }
        ));
        assert!(matches!(
            Value::boolean(true).infer_schema(),
            ValueSchema::Boolean
        ));
    }

    #[test]
    fn test_infer_object() {
        let obj = Object::from_iter(vec![
            ("name".to_string(), Value::text("Alice")),
            ("age".to_string(), Value::integer(30)),
        ]);
        let value = Value::Object(obj);
        let schema = value.infer_schema();

        assert!(schema.is_object_schema());
        if let ValueSchema::Object(obj_schema) = &schema {
            assert!(obj_schema.properties.contains_key("name"));
            assert!(obj_schema.properties.contains_key("age"));
        }
    }

    #[test]
    fn test_infer_array() {
        let arr = Array::from_vec(vec![Value::integer(1), Value::integer(2)]);
        let value = Value::Array(arr);
        let schema = value.infer_schema();

        assert!(schema.is_array_schema());
        if let ValueSchema::Array { items, .. } = &schema {
            assert!(matches!(items.as_ref(), ValueSchema::Integer { .. }));
        }
    }

    #[test]
    fn test_infer_from_samples() {
        let samples = vec![Value::integer(1), Value::integer(2), Value::integer(3)];
        let schema = Value::infer_schema_from_samples(&samples);
        assert!(matches!(schema, ValueSchema::Integer { .. }));

        let mixed = vec![Value::integer(1), Value::text("hello")];
        let schema = Value::infer_schema_from_samples(&mixed);
        assert!(matches!(schema, ValueSchema::OneOf(_)));
    }

    // ==================== Display Tests ====================

    #[test]
    fn test_schema_display() {
        assert_eq!(ValueSchema::any().to_string(), "any");
        assert_eq!(ValueSchema::null().to_string(), "null");
        assert_eq!(ValueSchema::boolean().to_string(), "boolean");
        assert_eq!(ValueSchema::integer().to_string(), "integer");
        assert_eq!(ValueSchema::string().to_string(), "string");
        assert_eq!(ValueSchema::string().nullable().to_string(), "string?");
    }
}
