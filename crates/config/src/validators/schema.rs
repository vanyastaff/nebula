//! JSON Schema-based configuration validator

use crate::core::config::json_type_name;
use crate::core::{ConfigError, ConfigResult, ConfigValidator};
use async_trait::async_trait;
use serde_json::Value;

/// Schema-based validator
pub struct SchemaValidator {
    /// JSON schema
    schema: Value,
    /// Whether to allow additional properties not in schema
    allow_additional: bool,
    /// Whether to coerce types when possible
    coerce_types: bool,
    /// Cache for compiled regex patterns (avoids recompilation on every validation)
    regex_cache: std::sync::Mutex<std::collections::HashMap<String, regex::Regex>>,
}

impl Clone for SchemaValidator {
    fn clone(&self) -> Self {
        Self {
            schema: self.schema.clone(),
            allow_additional: self.allow_additional,
            coerce_types: self.coerce_types,
            regex_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

impl std::fmt::Debug for SchemaValidator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchemaValidator")
            .field("schema", &self.schema)
            .field("allow_additional", &self.allow_additional)
            .field("coerce_types", &self.coerce_types)
            .finish()
    }
}

impl SchemaValidator {
    /// Create a new schema validator
    pub fn new(schema: Value) -> Self {
        Self {
            schema,
            allow_additional: true,
            coerce_types: false,
            regex_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Create from JSON schema string
    pub fn from_json(schema_json: &str) -> ConfigResult<Self> {
        let schema = serde_json::from_str(schema_json)?;
        Ok(Self::new(schema))
    }

    /// Set whether to allow additional properties
    #[must_use = "builder methods must be chained or built"]
    pub fn with_allow_additional(mut self, allow: bool) -> Self {
        self.allow_additional = allow;
        self
    }

    /// Set whether to coerce types
    #[must_use = "builder methods must be chained or built"]
    pub fn with_coerce_types(mut self, coerce: bool) -> Self {
        self.coerce_types = coerce;
        self
    }

    /// Recursive validation helper
    fn validate_recursive(&self, data: &Value, schema: &Value, path: &str) -> ConfigResult<()> {
        // Handle schema references ($ref)
        if let Some(schema_obj) = schema.as_object()
            && let Some(ref_val) = schema_obj.get("$ref")
            && let Some(ref_str) = ref_val.as_str()
        {
            return self.validate_ref(data, ref_str, path);
        }

        match schema {
            Value::Object(schema_obj) => self.validate_with_schema_object(data, schema_obj, path),
            Value::Bool(allow_all) => {
                if !allow_all {
                    Err(ConfigError::validation_error(
                        format!("Schema forbids any value at path '{}'", path),
                        Some(path.to_string()),
                    ))
                } else {
                    Ok(())
                }
            }
            _ => Err(ConfigError::validation_error(
                format!("Invalid schema format at path '{}'", path),
                Some(path.to_string()),
            )),
        }
    }

    /// Validate with schema object
    fn validate_with_schema_object(
        &self,
        data: &Value,
        schema_obj: &serde_json::Map<String, Value>,
        path: &str,
    ) -> ConfigResult<()> {
        // Check type
        if let Some(type_val) = schema_obj.get("type") {
            self.validate_type(data, type_val, path)?;
        }

        // Check enum
        if let Some(enum_val) = schema_obj.get("enum") {
            self.validate_enum(data, enum_val, path)?;
        }

        // Check const
        if let Some(const_val) = schema_obj.get("const")
            && data != const_val
        {
            return Err(ConfigError::validation_error(
                format!("Value at '{}' must be exactly {:?}", path, const_val),
                Some(path.to_string()),
            ));
        }

        // Type-specific validations
        match data {
            Value::Object(obj) => {
                self.validate_object(obj, schema_obj, path)?;
            }
            Value::Array(arr) => {
                self.validate_array(arr, schema_obj, path)?;
            }
            Value::String(s) => {
                self.validate_string(s, schema_obj, path)?;
            }
            Value::Number(n) => {
                self.validate_number(n, schema_obj, path)?;
            }
            _ => {}
        }

        Ok(())
    }

    /// Validate type constraint
    fn validate_type(&self, data: &Value, type_val: &Value, path: &str) -> ConfigResult<()> {
        let types: Vec<&str> = if let Some(type_str) = type_val.as_str() {
            vec![type_str]
        } else if let Some(type_arr) = type_val.as_array() {
            type_arr.iter().filter_map(|v| v.as_str()).collect()
        } else {
            return Ok(());
        };

        let data_type = json_type_name(data);

        if !types.contains(&data_type) {
            // Try type coercion if enabled
            if self.coerce_types && self.can_coerce(data, &types) {
                return Ok(());
            }

            return Err(ConfigError::validation_error(
                format!(
                    "Expected type {:?} at path '{}', got '{}'",
                    types, path, data_type
                ),
                Some(path.to_string()),
            ));
        }

        Ok(())
    }

    /// Validate enum constraint
    fn validate_enum(&self, data: &Value, enum_val: &Value, path: &str) -> ConfigResult<()> {
        if let Some(enum_arr) = enum_val.as_array()
            && !enum_arr.contains(data)
        {
            return Err(ConfigError::validation_error(
                format!("Value at '{}' must be one of {:?}", path, enum_arr),
                Some(path.to_string()),
            ));
        }
        Ok(())
    }

    /// Validate object
    fn validate_object(
        &self,
        obj: &serde_json::Map<String, Value>,
        schema_obj: &serde_json::Map<String, Value>,
        path: &str,
    ) -> ConfigResult<()> {
        // Check required fields
        if let Some(required_array) = schema_obj.get("required").and_then(Value::as_array) {
            for required_field in required_array {
                if let Some(field_name) = required_field.as_str()
                    && !obj.contains_key(field_name)
                {
                    return Err(ConfigError::validation_error(
                        format!("Required field '{}' missing at path '{}'", field_name, path),
                        Some(format!("{}.{}", path, field_name)),
                    ));
                }
            }
        }

        // Check properties
        if let Some(properties_obj) = schema_obj.get("properties").and_then(Value::as_object) {
            for (prop_name, prop_data) in obj {
                let new_path = if path.is_empty() {
                    prop_name.clone()
                } else {
                    format!("{}.{}", path, prop_name)
                };

                if let Some(prop_schema) = properties_obj.get(prop_name) {
                    self.validate_recursive(prop_data, prop_schema, &new_path)?;
                } else if !self.allow_additional {
                    // Check additionalProperties
                    let allow = schema_obj
                        .get("additionalProperties")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(self.allow_additional);

                    if !allow {
                        return Err(ConfigError::validation_error(
                            format!("Additional property '{}' not allowed", new_path),
                            Some(new_path),
                        ));
                    }
                }
            }
        }

        // Check property count
        if let Some(min) = schema_obj.get("minProperties").and_then(Value::as_u64)
            && (obj.len() as u64) < min
        {
            return Err(ConfigError::validation_error(
                format!("Object at '{}' must have at least {} properties", path, min),
                Some(path.to_string()),
            ));
        }

        if let Some(max) = schema_obj.get("maxProperties").and_then(Value::as_u64)
            && (obj.len() as u64) > max
        {
            return Err(ConfigError::validation_error(
                format!("Object at '{}' must have at most {} properties", path, max),
                Some(path.to_string()),
            ));
        }

        Ok(())
    }

    /// Validate array
    fn validate_array(
        &self,
        arr: &[Value],
        schema_obj: &serde_json::Map<String, Value>,
        path: &str,
    ) -> ConfigResult<()> {
        // Check items schema
        if let Some(items) = schema_obj.get("items") {
            if items.is_object() {
                for (i, item) in arr.iter().enumerate() {
                    let new_path = format!("{}[{}]", path, i);
                    self.validate_recursive(item, items, &new_path)?;
                }
            } else if let Some(items_array) = items.as_array() {
                // Tuple validation
                for (i, item) in arr.iter().enumerate() {
                    if let Some(item_schema) = items_array.get(i) {
                        let new_path = format!("{}[{}]", path, i);
                        self.validate_recursive(item, item_schema, &new_path)?;
                    }
                }
            }
        }

        // Check array length
        if let Some(min) = schema_obj.get("minItems").and_then(Value::as_u64)
            && (arr.len() as u64) < min
        {
            return Err(ConfigError::validation_error(
                format!("Array at '{}' must have at least {} items", path, min),
                Some(path.to_string()),
            ));
        }

        if let Some(max) = schema_obj.get("maxItems").and_then(Value::as_u64)
            && (arr.len() as u64) > max
        {
            return Err(ConfigError::validation_error(
                format!("Array at '{}' must have at most {} items", path, max),
                Some(path.to_string()),
            ));
        }

        // Check unique items
        if let Some(unique) = schema_obj.get("uniqueItems")
            && unique.as_bool().unwrap_or(false)
        {
            let mut seen = std::collections::HashSet::new();
            for item in arr {
                let item_str = serde_json::to_string(item).unwrap_or_default();
                if !seen.insert(item_str) {
                    return Err(ConfigError::validation_error(
                        format!("Array at '{}' must have unique items", path),
                        Some(path.to_string()),
                    ));
                }
            }
        }

        Ok(())
    }

    /// Validate string
    fn validate_string(
        &self,
        s: &str,
        schema_obj: &serde_json::Map<String, Value>,
        path: &str,
    ) -> ConfigResult<()> {
        // Check length constraints
        if let Some(min) = schema_obj.get("minLength").and_then(Value::as_u64)
            && (s.len() as u64) < min
        {
            return Err(ConfigError::validation_error(
                format!("String at '{}' must be at least {} characters", path, min),
                Some(path.to_string()),
            ));
        }

        if let Some(max) = schema_obj.get("maxLength").and_then(Value::as_u64)
            && (s.len() as u64) > max
        {
            return Err(ConfigError::validation_error(
                format!("String at '{}' must be at most {} characters", path, max),
                Some(path.to_string()),
            ));
        }

        // Check pattern (with compiled regex cache)
        if let Some(pattern_str) = schema_obj.get("pattern").and_then(Value::as_str) {
            let matches = {
                let mut cache = self.regex_cache.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(re) = cache.get(pattern_str) {
                    Some(re.is_match(s))
                } else {
                    match regex::Regex::new(pattern_str) {
                        Ok(re) => {
                            let result = re.is_match(s);
                            cache.insert(pattern_str.to_string(), re);
                            Some(result)
                        }
                        Err(_) => {
                            nebula_log::warn!("Invalid regex pattern in schema: {}", pattern_str);
                            None
                        }
                    }
                }
            };
            if matches == Some(false) {
                return Err(ConfigError::validation_error(
                    format!("String at '{}' must match pattern '{}'", path, pattern_str),
                    Some(path.to_string()),
                ));
            }
        }

        // Check format
        if let Some(format_str) = schema_obj.get("format").and_then(Value::as_str) {
            self.validate_string_format(s, format_str, path)?;
        }

        Ok(())
    }

    /// Validate number
    fn validate_number(
        &self,
        n: &serde_json::Number,
        schema_obj: &serde_json::Map<String, Value>,
        path: &str,
    ) -> ConfigResult<()> {
        let value = n.as_f64().unwrap_or(0.0);

        // Check minimum
        if let Some(min) = schema_obj.get("minimum").and_then(Value::as_f64) {
            let exclusive = schema_obj
                .get("exclusiveMinimum")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if (exclusive && value <= min) || (!exclusive && value < min) {
                return Err(ConfigError::validation_error(
                    format!(
                        "Number at '{}' must be {} {}",
                        path,
                        if exclusive {
                            "greater than"
                        } else {
                            "at least"
                        },
                        min
                    ),
                    Some(path.to_string()),
                ));
            }
        }

        // Check maximum
        if let Some(max) = schema_obj.get("maximum").and_then(Value::as_f64) {
            let exclusive = schema_obj
                .get("exclusiveMaximum")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if (exclusive && value >= max) || (!exclusive && value > max) {
                return Err(ConfigError::validation_error(
                    format!(
                        "Number at '{}' must be {} {}",
                        path,
                        if exclusive { "less than" } else { "at most" },
                        max
                    ),
                    Some(path.to_string()),
                ));
            }
        }

        // Check multipleOf
        if let Some(divisor) = schema_obj.get("multipleOf").and_then(Value::as_f64)
            && divisor != 0.0
        {
            let remainder = value % divisor;
            if remainder.abs() > f64::EPSILON {
                return Err(ConfigError::validation_error(
                    format!("Number at '{}' must be a multiple of {}", path, divisor),
                    Some(path.to_string()),
                ));
            }
        }

        Ok(())
    }

    /// Validate string format
    fn validate_string_format(&self, s: &str, format: &str, path: &str) -> ConfigResult<()> {
        let is_valid = match format {
            "email" => self.is_valid_email(s),
            "uri" | "url" => self.is_valid_url(s),
            "ipv4" => self.is_valid_ipv4(s),
            "ipv6" => self.is_valid_ipv6(s),
            "uuid" => self.is_valid_uuid(s),
            "date" => self.is_valid_date(s),
            "date-time" => self.is_valid_datetime(s),
            "time" => self.is_valid_time(s),
            "hostname" => self.is_valid_hostname(s),
            _ => true, // Unknown format, skip validation
        };

        if !is_valid {
            return Err(ConfigError::validation_error(
                format!("String at '{}' is not a valid {}", path, format),
                Some(path.to_string()),
            ));
        }

        Ok(())
    }

    /// Validate reference
    fn validate_ref(&self, data: &Value, ref_str: &str, path: &str) -> ConfigResult<()> {
        // Simple implementation - just validate against definitions
        if let Some(def_name) = ref_str.strip_prefix("#/definitions/")
            && let Some(definitions) = self.schema.get("definitions")
            && let Some(def_schema) = definitions.get(def_name)
        {
            return self.validate_recursive(data, def_schema, path);
        }

        Err(ConfigError::validation_error(
            format!("Cannot resolve reference '{ref_str}' at path '{path}'"),
            Some(path.to_string()),
        ))
    }

    /// Check if value can be coerced to target types (no String allocation for comparison)
    fn can_coerce(&self, value: &Value, target_types: &[&str]) -> bool {
        match value {
            Value::String(s) => {
                // String to number
                if target_types.contains(&"number") && s.parse::<f64>().is_ok() {
                    return true;
                }
                // String to boolean
                if target_types.contains(&"boolean")
                    && (s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("false"))
                {
                    return true;
                }
            }
            Value::Number(_) => {
                if target_types.contains(&"string") {
                    return true;
                }
            }
            Value::Bool(_) => {
                if target_types.contains(&"string") {
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    // Format validators
    fn is_valid_email(&self, s: &str) -> bool {
        s.contains('@') && s.contains('.')
    }

    fn is_valid_url(&self, s: &str) -> bool {
        s.starts_with("http://") || s.starts_with("https://") || s.starts_with("ftp://")
    }

    fn is_valid_ipv4(&self, s: &str) -> bool {
        s.parse::<std::net::Ipv4Addr>().is_ok()
    }

    fn is_valid_ipv6(&self, s: &str) -> bool {
        s.parse::<std::net::Ipv6Addr>().is_ok()
    }

    fn is_valid_uuid(&self, s: &str) -> bool {
        uuid::Uuid::parse_str(s).is_ok()
    }

    fn is_valid_date(&self, s: &str) -> bool {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
    }

    fn is_valid_datetime(&self, s: &str) -> bool {
        chrono::DateTime::parse_from_rfc3339(s).is_ok()
    }

    fn is_valid_time(&self, s: &str) -> bool {
        chrono::NaiveTime::parse_from_str(s, "%H:%M:%S").is_ok()
    }

    fn is_valid_hostname(&self, s: &str) -> bool {
        !s.is_empty()
            && s.len() <= 253
            && s.chars()
                .all(|c| c.is_alphanumeric() || c == '.' || c == '-')
    }
}

#[async_trait]
impl ConfigValidator for SchemaValidator {
    async fn validate(&self, data: &Value) -> ConfigResult<()> {
        self.validate_recursive(data, &self.schema, "")
    }

    fn schema(&self) -> Option<Value> {
        Some(self.schema.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ConfigValidator;
    use serde_json::json;

    #[tokio::test]
    async fn test_schema_new_and_from_json() {
        let schema = json!({"type": "object"});
        let v = SchemaValidator::new(schema.clone());
        assert_eq!(v.schema(), Some(schema));

        let v2 = SchemaValidator::from_json(r#"{"type": "string"}"#).unwrap();
        assert_eq!(v2.schema(), Some(json!({"type": "string"})));

        assert!(SchemaValidator::from_json("not valid json {{{").is_err());
    }

    #[tokio::test]
    async fn test_validate_type_string() {
        let v = SchemaValidator::new(json!({"type": "string"}));

        assert!(v.validate(&json!("hello")).await.is_ok());
        assert!(v.validate(&json!(42)).await.is_err());
        assert!(v.validate(&json!(true)).await.is_err());
        assert!(v.validate(&json!(null)).await.is_err());
    }

    #[tokio::test]
    async fn test_validate_type_number_and_boolean() {
        let v_num = SchemaValidator::new(json!({"type": "number"}));
        assert!(v_num.validate(&json!(42)).await.is_ok());
        assert!(v_num.validate(&json!(3.14)).await.is_ok());
        assert!(v_num.validate(&json!("hello")).await.is_err());

        let v_bool = SchemaValidator::new(json!({"type": "boolean"}));
        assert!(v_bool.validate(&json!(true)).await.is_ok());
        assert!(v_bool.validate(&json!(false)).await.is_ok());
        assert!(v_bool.validate(&json!(1)).await.is_err());
    }

    #[tokio::test]
    async fn test_validate_type_array_of_types() {
        let v = SchemaValidator::new(json!({"type": ["string", "number"]}));
        assert!(v.validate(&json!("hello")).await.is_ok());
        assert!(v.validate(&json!(42)).await.is_ok());
        assert!(v.validate(&json!(true)).await.is_err());
        assert!(v.validate(&json!(null)).await.is_err());
    }

    #[tokio::test]
    async fn test_validate_type_coercion() {
        let v = SchemaValidator::new(json!({"type": "number"})).with_coerce_types(true);

        // String "42" coercible to number
        assert!(v.validate(&json!("42")).await.is_ok());
        // String "hello" not coercible
        assert!(v.validate(&json!("hello")).await.is_err());

        let v_bool = SchemaValidator::new(json!({"type": "boolean"})).with_coerce_types(true);
        assert!(v_bool.validate(&json!("true")).await.is_ok());
        assert!(v_bool.validate(&json!("FALSE")).await.is_ok());
        assert!(v_bool.validate(&json!("maybe")).await.is_err());

        // Without coercion, string "42" fails for number type
        let v_strict = SchemaValidator::new(json!({"type": "number"}));
        assert!(v_strict.validate(&json!("42")).await.is_err());
    }

    #[tokio::test]
    async fn test_validate_string_constraints() {
        let v = SchemaValidator::new(json!({
            "type": "string",
            "minLength": 3,
            "maxLength": 10,
            "pattern": "^[a-z]+$"
        }));

        assert!(v.validate(&json!("hello")).await.is_ok());
        assert!(v.validate(&json!("ab")).await.is_err()); // too short
        assert!(v.validate(&json!("abcdefghijk")).await.is_err()); // too long
        assert!(v.validate(&json!("Hello")).await.is_err()); // uppercase fails pattern
        assert!(v.validate(&json!("abc123")).await.is_err()); // digits fail pattern
    }

    #[tokio::test]
    async fn test_validate_string_format() {
        let v = SchemaValidator::new(json!({
            "type": "object",
            "properties": {
                "email": {"type": "string", "format": "email"},
                "url": {"type": "string", "format": "uri"},
                "ip4": {"type": "string", "format": "ipv4"},
                "ip6": {"type": "string", "format": "ipv6"},
                "id": {"type": "string", "format": "uuid"},
                "date": {"type": "string", "format": "date"},
                "dt": {"type": "string", "format": "date-time"},
                "time": {"type": "string", "format": "time"},
                "host": {"type": "string", "format": "hostname"}
            }
        }));

        let valid = json!({
            "email": "test@example.com",
            "url": "https://example.com",
            "ip4": "192.168.1.1",
            "ip6": "::1",
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "date": "2025-01-15",
            "dt": "2025-01-15T10:30:00Z",
            "time": "10:30:00",
            "host": "example.com"
        });
        assert!(v.validate(&valid).await.is_ok());

        // Invalid email
        let bad_email = json!({"email": "not-an-email"});
        assert!(v.validate(&bad_email).await.is_err());

        // Invalid IPv4
        let bad_ip = json!({"ip4": "999.999.999.999"});
        assert!(v.validate(&bad_ip).await.is_err());

        // Invalid UUID
        let bad_uuid = json!({"id": "not-a-uuid"});
        assert!(v.validate(&bad_uuid).await.is_err());
    }

    #[tokio::test]
    async fn test_validate_number_constraints() {
        let v = SchemaValidator::new(json!({
            "type": "number",
            "minimum": 0,
            "maximum": 100
        }));

        assert!(v.validate(&json!(0)).await.is_ok());
        assert!(v.validate(&json!(50)).await.is_ok());
        assert!(v.validate(&json!(100)).await.is_ok());
        assert!(v.validate(&json!(-1)).await.is_err());
        assert!(v.validate(&json!(101)).await.is_err());

        // Exclusive bounds
        let v_excl = SchemaValidator::new(json!({
            "type": "number",
            "minimum": 0,
            "exclusiveMinimum": true,
            "maximum": 100,
            "exclusiveMaximum": true
        }));
        assert!(v_excl.validate(&json!(0)).await.is_err()); // exclusive
        assert!(v_excl.validate(&json!(1)).await.is_ok());
        assert!(v_excl.validate(&json!(100)).await.is_err()); // exclusive
        assert!(v_excl.validate(&json!(99)).await.is_ok());

        // multipleOf
        let v_mult = SchemaValidator::new(json!({
            "type": "number",
            "multipleOf": 3
        }));
        assert!(v_mult.validate(&json!(9)).await.is_ok());
        assert!(v_mult.validate(&json!(10)).await.is_err());
    }

    #[tokio::test]
    async fn test_validate_object_required_and_properties() {
        let v = SchemaValidator::new(json!({
            "type": "object",
            "required": ["name", "age"],
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "number"}
            }
        }));

        let valid = json!({"name": "Alice", "age": 30});
        assert!(v.validate(&valid).await.is_ok());

        // Missing required field
        let missing_age = json!({"name": "Alice"});
        assert!(v.validate(&missing_age).await.is_err());

        // Wrong type for property
        let wrong_type = json!({"name": 123, "age": 30});
        assert!(v.validate(&wrong_type).await.is_err());

        // Extra properties allowed by default
        let extra = json!({"name": "Alice", "age": 30, "email": "a@b.com"});
        assert!(v.validate(&extra).await.is_ok());
    }

    #[tokio::test]
    async fn test_validate_object_additional_properties() {
        let v = SchemaValidator::new(json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"}
            },
            "additionalProperties": false
        }))
        .with_allow_additional(false);

        let valid = json!({"name": "Alice"});
        assert!(v.validate(&valid).await.is_ok());

        let extra = json!({"name": "Alice", "extra": true});
        assert!(v.validate(&extra).await.is_err());

        // with_allow_additional(true) overrides
        let v_allow = SchemaValidator::new(json!({
            "type": "object",
            "properties": {"name": {"type": "string"}}
        }))
        .with_allow_additional(true);
        let extra2 = json!({"name": "Alice", "extra": true});
        assert!(v_allow.validate(&extra2).await.is_ok());
    }

    #[tokio::test]
    async fn test_validate_array_constraints() {
        // Items schema (all same type)
        let v = SchemaValidator::new(json!({
            "type": "array",
            "items": {"type": "number"},
            "minItems": 1,
            "maxItems": 3,
            "uniqueItems": true
        }));

        assert!(v.validate(&json!([1, 2, 3])).await.is_ok());
        assert!(v.validate(&json!([])).await.is_err()); // minItems
        assert!(v.validate(&json!([1, 2, 3, 4])).await.is_err()); // maxItems
        assert!(v.validate(&json!([1, 1, 2])).await.is_err()); // uniqueItems
        assert!(v.validate(&json!(["a"])).await.is_err()); // wrong item type

        // Tuple validation
        let v_tuple = SchemaValidator::new(json!({
            "type": "array",
            "items": [{"type": "string"}, {"type": "number"}]
        }));
        assert!(v_tuple.validate(&json!(["hello", 42])).await.is_ok());
        assert!(v_tuple.validate(&json!([42, "hello"])).await.is_err());
    }

    #[tokio::test]
    async fn test_validate_enum_const_ref_boolean_schema() {
        // Enum
        let v_enum = SchemaValidator::new(json!({"enum": ["red", "green", "blue"]}));
        assert!(v_enum.validate(&json!("red")).await.is_ok());
        assert!(v_enum.validate(&json!("yellow")).await.is_err());

        // Const
        let v_const = SchemaValidator::new(json!({"const": 42}));
        assert!(v_const.validate(&json!(42)).await.is_ok());
        assert!(v_const.validate(&json!(43)).await.is_err());

        // $ref to definitions
        let v_ref = SchemaValidator::new(json!({
            "definitions": {
                "port": {"type": "number", "minimum": 1, "maximum": 65535}
            },
            "type": "object",
            "properties": {
                "port": {"$ref": "#/definitions/port"}
            }
        }));
        assert!(v_ref.validate(&json!({"port": 8080})).await.is_ok());
        assert!(v_ref.validate(&json!({"port": 0})).await.is_err());
        assert!(v_ref.validate(&json!({"port": 99999})).await.is_err());

        // Unresolvable $ref
        let v_bad_ref = SchemaValidator::new(json!({"$ref": "#/definitions/missing"}));
        assert!(v_bad_ref.validate(&json!("anything")).await.is_err());

        // Boolean schema
        let v_true = SchemaValidator::new(json!(true));
        assert!(v_true.validate(&json!("anything")).await.is_ok());

        let v_false = SchemaValidator::new(json!(false));
        assert!(v_false.validate(&json!("anything")).await.is_err());
    }
}
