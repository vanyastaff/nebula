//! Serialization support for nebula-memory
//!
//! This module provides extension traits that allow integrating
//! the memory management system with various serialization formats.

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::{
    boxed::Box,
    collections::BTreeMap,
    string::{String, ToString},
    sync::Arc,
    vec,
    vec::Vec,
};
use core::fmt;
#[cfg(feature = "std")]
use std::{
    boxed::Box,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};

use crate::core::error::{MemoryError, MemoryResult};
use crate::extensions::MemoryExtension;

/// Possible serialization formats
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SerializationFormat {
    /// JSON format
    Json,
    /// Bincode binary format
    Bincode,
    /// MessagePack format
    MessagePack,
    /// CBOR format
    Cbor,
    /// Custom format
    Custom(&'static str),
}

impl fmt::Display for SerializationFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json => write!(f, "json"),
            Self::Bincode => write!(f, "bincode"),
            Self::MessagePack => write!(f, "messagepack"),
            Self::Cbor => write!(f, "cbor"),
            Self::Custom(name) => write!(f, "custom({})", name),
        }
    }
}

/// Trait for serializing data
pub trait Serializer: Send + Sync {
    /// Get the format used by this serializer
    fn format(&self) -> SerializationFormat;

    /// Serialize data to bytes
    fn serialize(&self, value: &dyn SerializableValue) -> MemoryResult<Vec<u8>>;

    /// Deserialize data from bytes
    fn deserialize(&self, data: &[u8], type_hint: &str)
        -> MemoryResult<Box<dyn SerializableValue>>;
}

/// Trait for values that can be serialized
pub trait SerializableValue: Send + Sync {
    /// Get the type name of this value
    fn type_name(&self) -> &str;

    /// Serialize this value to JSON (implementation-dependent)
    fn to_json(&self) -> MemoryResult<String> {
        Err(MemoryError::NotSupported {
            feature: "JSON serialization",
            context: Some(format!("Type {} does not support JSON serialization", self.type_name())),
        })
    }

    /// Clone this value
    fn clone_value(&self) -> Box<dyn SerializableValue>;

    /// Cast this value to Any for dynamic downcasting
    fn as_any(&self) -> &dyn core::any::Any;

    /// Compare this value with another value for equality
    fn equals(&self, other: &dyn SerializableValue) -> bool {
        // Default implementation just compares type names
        self.type_name() == other.type_name()
    }
}

/// A serializable string value
#[derive(Debug, Clone)]
pub struct StringValue(pub String);

impl SerializableValue for StringValue {
    fn type_name(&self) -> &str {
        "string"
    }

    fn to_json(&self) -> MemoryResult<String> {
        // Simple JSON string escaping
        let escaped = self
            .0
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t");
        Ok(format!("\"{}\"", escaped))
    }

    fn clone_value(&self) -> Box<dyn SerializableValue> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn equals(&self, other: &dyn SerializableValue) -> bool {
        if let Some(other) = other.as_any().downcast_ref::<Self>() {
            self.0 == other.0
        } else {
            false
        }
    }
}

/// A serializable number value
#[derive(Debug, Clone, Copy)]
pub struct NumberValue(pub f64);

impl SerializableValue for NumberValue {
    fn type_name(&self) -> &str {
        "number"
    }

    fn to_json(&self) -> MemoryResult<String> {
        Ok(self.0.to_string())
    }

    fn clone_value(&self) -> Box<dyn SerializableValue> {
        Box::new(*self)
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn equals(&self, other: &dyn SerializableValue) -> bool {
        if let Some(other) = other.as_any().downcast_ref::<Self>() {
            (self.0 - other.0).abs() < f64::EPSILON
        } else {
            false
        }
    }
}

/// A serializable boolean value
#[derive(Debug, Clone, Copy)]
pub struct BooleanValue(pub bool);

impl SerializableValue for BooleanValue {
    fn type_name(&self) -> &str {
        "boolean"
    }

    fn to_json(&self) -> MemoryResult<String> {
        Ok(if self.0 { "true".to_string() } else { "false".to_string() })
    }

    fn clone_value(&self) -> Box<dyn SerializableValue> {
        Box::new(*self)
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn equals(&self, other: &dyn SerializableValue) -> bool {
        if let Some(other) = other.as_any().downcast_ref::<Self>() {
            self.0 == other.0
        } else {
            false
        }
    }
}

/// A serializable null value
#[derive(Debug, Clone, Copy)]
pub struct NullValue;

impl SerializableValue for NullValue {
    fn type_name(&self) -> &str {
        "null"
    }

    fn to_json(&self) -> MemoryResult<String> {
        Ok("null".to_string())
    }

    fn clone_value(&self) -> Box<dyn SerializableValue> {
        Box::new(*self)
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn equals(&self, other: &dyn SerializableValue) -> bool {
        other.as_any().downcast_ref::<Self>().is_some()
    }
}

/// A simple JSON serializer implementation
pub struct JsonSerializer;

impl Serializer for JsonSerializer {
    fn format(&self) -> SerializationFormat {
        SerializationFormat::Json
    }

    fn serialize(&self, value: &dyn SerializableValue) -> MemoryResult<Vec<u8>> {
        let json = value.to_json()?;
        Ok(json.into_bytes())
    }

    fn deserialize(
        &self,
        data: &[u8],
        type_hint: &str,
    ) -> MemoryResult<Box<dyn SerializableValue>> {
        let json = core::str::from_utf8(data).map_err(|_| MemoryError::InvalidConfig {
            reason: "Invalid UTF-8 in JSON data".to_string(),
        })?;

        // Very simplified JSON parsing - just for demonstration
        let json = json.trim();

        match type_hint {
            "string" => {
                if json.starts_with('"') && json.ends_with('"') {
                    let content = json[1..json.len() - 1].to_string();
                    Ok(Box::new(StringValue(content)))
                } else {
                    Err(MemoryError::InvalidConfig { reason: "Expected JSON string".to_string() })
                }
            },
            "number" => {
                let num = json.parse::<f64>().map_err(|_| MemoryError::InvalidConfig {
                    reason: "Invalid JSON number".to_string(),
                })?;
                Ok(Box::new(NumberValue(num)))
            },
            "boolean" => match json {
                "true" => Ok(Box::new(BooleanValue(true))),
                "false" => Ok(Box::new(BooleanValue(false))),
                _ => Err(MemoryError::InvalidConfig { reason: "Invalid JSON boolean".to_string() }),
            },
            "null" => {
                if json == "null" {
                    Ok(Box::new(NullValue))
                } else {
                    Err(MemoryError::InvalidConfig { reason: "Expected JSON null".to_string() })
                }
            },
            _ => Err(MemoryError::NotSupported {
                feature: "JSON deserialization",
                context: Some(format!("Type {} not supported", type_hint)),
            }),
        }
    }
}

/// Serialization extension that manages multiple serializers
pub struct SerializationExtension {
    /// Registered serializers
    serializers: Vec<Box<dyn Serializer>>,
}

impl SerializationExtension {
    /// Create a new serialization extension
    pub fn new() -> Self {
        Self { serializers: Vec::new() }
    }

    /// Register a serializer
    pub fn register_serializer(&mut self, serializer: impl Serializer + 'static) {
        self.serializers.push(Box::new(serializer));
    }

    /// Get a serializer by format
    pub fn get_serializer(&self, format: SerializationFormat) -> Option<&dyn Serializer> {
        self.serializers.iter().find(|s| s.format() == format).map(|s| s.as_ref())
    }

    /// Serialize a value with the specified format
    pub fn serialize(
        &self,
        value: &dyn SerializableValue,
        format: SerializationFormat,
    ) -> MemoryResult<Vec<u8>> {
        match self.get_serializer(format) {
            Some(serializer) => serializer.serialize(value),
            None => Err(MemoryError::NotSupported {
                feature: "Serialization format",
                context: Some(format!("{} is not supported", format)),
            }),
        }
    }

    /// Deserialize a value with the specified format
    pub fn deserialize(
        &self,
        data: &[u8],
        type_hint: &str,
        format: SerializationFormat,
    ) -> MemoryResult<Box<dyn SerializableValue>> {
        match self.get_serializer(format) {
            Some(serializer) => serializer.deserialize(data, type_hint),
            None => Err(MemoryError::NotSupported {
                feature: "Deserialization format",
                context: Some(format!("{} is not supported", format)),
            }),
        }
    }
}

impl Default for SerializationExtension {
    fn default() -> Self {
        let mut extension = Self::new();
        extension.register_serializer(JsonSerializer);
        extension
    }
}

impl MemoryExtension for SerializationExtension {
    fn name(&self) -> &str {
        "serialization"
    }

    fn version(&self) -> &str {
        "1.0.0"
    }

    fn category(&self) -> &str {
        "serialization"
    }

    fn tags(&self) -> Vec<&str> {
        vec!["serialization", "io"]
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

/// Helper to get the current global serialization extension
pub fn global_serialization() -> Option<Arc<SerializationExtension>> {
    use crate::extensions::GlobalExtensions;

    if let Some(ext) = GlobalExtensions::get("serialization") {
        if let Some(ser_ext) = ext.as_any().downcast_ref::<SerializationExtension>() {
            // Создаем новый экземпляр с теми же сериализаторами
            let mut new_ext = SerializationExtension::new();

            // Копируем зарегистрированные сериализаторы через вызов register_serializer
            for serializer in &ser_ext.serializers {
                // Регистрируем JSON сериализатор по умолчанию
                if serializer.format() == SerializationFormat::Json {
                    new_ext.register_serializer(JsonSerializer);
                }
                // Другие форматы будут недоступны, но в данном контексте
                // это нормально, т.к. мы не можем клонировать сериализаторы
                // напрямую
            }

            return Some(Arc::new(new_ext));
        }
    }
    None
}

/// Initialize the global serialization extension
pub fn init_global_serialization() -> MemoryResult<()> {
    use crate::extensions::GlobalExtensions;

    let extension = SerializationExtension::default();
    GlobalExtensions::register(extension)
}

/// Serialize a value using the global serialization extension
pub fn serialize(
    value: &dyn SerializableValue,
    format: SerializationFormat,
) -> MemoryResult<Vec<u8>> {
    match global_serialization() {
        Some(extension) => extension.serialize(value, format),
        None => Err(MemoryError::NotSupported {
            feature: "Serialization",
            context: Some("Global serialization extension not initialized".to_string()),
        }),
    }
}

/// Deserialize a value using the global serialization extension
pub fn deserialize(
    data: &[u8],
    type_hint: &str,
    format: SerializationFormat,
) -> MemoryResult<Box<dyn SerializableValue>> {
    match global_serialization() {
        Some(extension) => extension.deserialize(data, type_hint, format),
        None => Err(MemoryError::NotSupported {
            feature: "Deserialization",
            context: Some("Global serialization extension not initialized".to_string()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_value() {
        let value = StringValue("Hello, world!".to_string());
        assert_eq!(value.type_name(), "string");
        assert_eq!(value.to_json().unwrap(), "\"Hello, world!\"");

        let clone = value.clone_value();
        assert!(value.equals(clone.as_ref()));
    }

    #[test]
    fn test_number_value() {
        let value = NumberValue(42.5);
        assert_eq!(value.type_name(), "number");
        assert_eq!(value.to_json().unwrap(), "42.5");

        let clone = value.clone_value();
        assert!(value.equals(clone.as_ref()));
    }

    #[test]
    fn test_boolean_value() {
        let value = BooleanValue(true);
        assert_eq!(value.type_name(), "boolean");
        assert_eq!(value.to_json().unwrap(), "true");

        let clone = value.clone_value();
        assert!(value.equals(clone.as_ref()));
    }

    #[test]
    fn test_null_value() {
        let value = NullValue;
        assert_eq!(value.type_name(), "null");
        assert_eq!(value.to_json().unwrap(), "null");

        let clone = value.clone_value();
        assert!(value.equals(clone.as_ref()));
    }

    #[test]
    fn test_json_serializer() {
        let serializer = JsonSerializer;
        assert_eq!(serializer.format(), SerializationFormat::Json);

        let string_value = StringValue("Test".to_string());
        let serialized = serializer.serialize(&string_value).unwrap();
        let deserialized = serializer.deserialize(&serialized, "string").unwrap();

        assert!(string_value.equals(deserialized.as_ref()));
    }

    #[test]
    fn test_serialization_extension() {
        let mut extension = SerializationExtension::new();
        extension.register_serializer(JsonSerializer);

        assert!(extension.get_serializer(SerializationFormat::Json).is_some());
        assert!(extension.get_serializer(SerializationFormat::Bincode).is_none());

        let value = StringValue("Extension test".to_string());
        let serialized = extension.serialize(&value, SerializationFormat::Json).unwrap();
        let deserialized =
            extension.deserialize(&serialized, "string", SerializationFormat::Json).unwrap();

        assert!(value.equals(deserialized.as_ref()));
    }
}
