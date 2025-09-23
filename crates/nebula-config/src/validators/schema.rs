//! JSON Schema-based configuration validator

use crate::core::{ConfigError, ConfigResult, ConfigValidator};
use async_trait::async_trait;
use serde_json::Value;

/// Schema-based validator
#[derive(Debug, Clone)]
pub struct SchemaValidator {
    /// JSON schema
    schema: Value,
    /// Whether to allow additional properties not in schema
    allow_additional: bool,
    /// Whether to coerce types when possible
    coerce_types: bool,
}

impl SchemaValidator {
    /// Create a new schema validator
    pub fn new(schema: Value) -> Self {
        Self {
            schema,
            allow_additional: true,
            coerce_types: false,
        }
    }

    /// Create from JSON schema string
    pub fn from_json(schema_json: &str) -> ConfigResult<Self> {
        let schema = serde_json::from_str(schema_json)?;
        Ok(Self::new(schema))
    }

    /// Set whether to allow additional properties
    pub fn with_allow_additional(mut self, allow: bool) -> Self {
        self.allow_additional = allow;
        self
    }

    /// Set whether to coerce types
    pub fn with_coerce_types(mut self, coerce: bool) -> Self {
        self.coerce_types = coerce;
        self
    }

    /// Recursive validation helper
    fn validate_recursive(&self, data: &Value, schema: &Value, path: &str) -> ConfigResult<()> {
        // Handle schema references ($ref)
        if let Some(schema_obj) = schema.as_object() {
            if let Some(ref_val) = schema_obj.get("$ref") {
                if let Some(ref_str) = ref_val.as_str() {
                    return self.validate_ref(data, ref_str, path);
                }
            }
        }

        match schema {
            Value::Object(schema_obj) => {
                self.validate_with_schema_object(data, schema_obj, path)
            }
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
            _ => {
                Err(ConfigError::validation_error(
                    format!("Invalid schema format at path '{}'", path),
                    Some(path.to_string()),
                ))
            }
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
        if let Some(const_val) = schema_obj.get("const") {
            if data != const_val {
                return Err(ConfigError::validation_error(
                    format!("Value at '{}' must be exactly {:?}", path, const_val),
                    Some(path.to_string()),
                ));
            }
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
        let types = if let Some(type_str) = type_val.as_str() {
            vec![type_str.to_string()]
        } else if let Some(type_arr) = type_val.as_array() {
            type_arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        } else {
            return Ok(());
        };

        let data_type = self.get_json_type(data);

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
        if let Some(enum_arr) = enum_val.as_array() {
            if !enum_arr.contains(data) {
                return Err(ConfigError::validation_error(
                    format!(
                        "Value at '{}' must be one of {:?}",
                        path,
                        enum_arr
                    ),
                    Some(path.to_string()),
                ));
            }
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
        if let Some(required) = schema_obj.get("required") {
            if let Some(required_array) = required.as_array() {
                for required_field in required_array {
                    if let Some(field_name) = required_field.as_str() {
                        if !obj.contains_key(field_name) {
                            return Err(ConfigError::validation_error(
                                format!(
                                    "Required field '{}' missing at path '{}'",
                                    field_name, path
                                ),
                                Some(format!("{}.{}", path, field_name)),
                            ));
                        }
                    }
                }
            }
        }

        // Check properties
        if let Some(properties) = schema_obj.get("properties") {
            if let Some(properties_obj) = properties.as_object() {
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
        }

        // Check property count
        if let Some(min_props) = schema_obj.get("minProperties") {
            if let Some(min) = min_props.as_u64() {
                if (obj.len() as u64) < min {
                    return Err(ConfigError::validation_error(
                        format!(
                            "Object at '{}' must have at least {} properties",
                            path, min
                        ),
                        Some(path.to_string()),
                    ));
                }
            }
        }

        if let Some(max_props) = schema_obj.get("maxProperties") {
            if let Some(max) = max_props.as_u64() {
                if (obj.len() as u64) > max {
                    return Err(ConfigError::validation_error(
                        format!(
                            "Object at '{}' must have at most {} properties",
                            path, max
                        ),
                        Some(path.to_string()),
                    ));
                }
            }
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
            if let Some(_items_schema) = items.as_object() {
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
        if let Some(min_items) = schema_obj.get("minItems") {
            if let Some(min) = min_items.as_u64() {
                if (arr.len() as u64) < min {
                    return Err(ConfigError::validation_error(
                        format!("Array at '{}' must have at least {} items", path, min),
                        Some(path.to_string()),
                    ));
                }
            }
        }

        if let Some(max_items) = schema_obj.get("maxItems") {
            if let Some(max) = max_items.as_u64() {
                if (arr.len() as u64) > max {
                    return Err(ConfigError::validation_error(
                        format!("Array at '{}' must have at most {} items", path, max),
                        Some(path.to_string()),
                    ));
                }
            }
        }

        // Check unique items
        if let Some(unique) = schema_obj.get("uniqueItems") {
            if unique.as_bool().unwrap_or(false) {
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
        if let Some(min_length) = schema_obj.get("minLength") {
            if let Some(min) = min_length.as_u64() {
                if (s.len() as u64) < min {
                    return Err(ConfigError::validation_error(
                        format!(
                            "String at '{}' must be at least {} characters",
                            path, min
                        ),
                        Some(path.to_string()),
                    ));
                }
            }
        }

        if let Some(max_length) = schema_obj.get("maxLength") {
            if let Some(max) = max_length.as_u64() {
                if (s.len() as u64) > max {
                    return Err(ConfigError::validation_error(
                        format!(
                            "String at '{}' must be at most {} characters",
                            path, max
                        ),
                        Some(path.to_string()),
                    ));
                }
            }
        }

        // Check pattern
        if let Some(pattern) = schema_obj.get("pattern") {
            if let Some(pattern_str) = pattern.as_str() {
                match regex::Regex::new(pattern_str) {
                    Ok(re) => {
                        if !re.is_match(s) {
                            return Err(ConfigError::validation_error(
                                format!(
                                    "String at '{}' must match pattern '{}'",
                                    path, pattern_str
                                ),
                                Some(path.to_string()),
                            ));
                        }
                    }
                    Err(_) => {
                        nebula_log::warn!("Invalid regex pattern in schema: {}", pattern_str);
                    }
                }
            }
        }

        // Check format
        if let Some(format) = schema_obj.get("format") {
            if let Some(format_str) = format.as_str() {
                self.validate_string_format(s, format_str, path)?;
            }
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
        if let Some(minimum) = schema_obj.get("minimum") {
            if let Some(min) = minimum.as_f64() {
                let exclusive = schema_obj
                    .get("exclusiveMinimum")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                if (exclusive && value <= min) || (!exclusive && value < min) {
                    return Err(ConfigError::validation_error(
                        format!(
                            "Number at '{}' must be {} {}",
                            path,
                            if exclusive { "greater than" } else { "at least" },
                            min
                        ),
                        Some(path.to_string()),
                    ));
                }
            }
        }

        // Check maximum
        if let Some(maximum) = schema_obj.get("maximum") {
            if let Some(max) = maximum.as_f64() {
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
        }

        // Check multipleOf
        if let Some(multiple_of) = schema_obj.get("multipleOf") {
            if let Some(divisor) = multiple_of.as_f64() {
                if divisor != 0.0 {
                    let remainder = value % divisor;
                    if remainder.abs() > f64::EPSILON {
                        return Err(ConfigError::validation_error(
                            format!(
                                "Number at '{}' must be a multiple of {}",
                                path, divisor
                            ),
                            Some(path.to_string()),
                        ));
                    }
                }
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
        if ref_str.starts_with("#/definitions/") {
            let def_name = &ref_str[14..];
            if let Some(definitions) = self.schema.get("definitions") {
                if let Some(def_schema) = definitions.get(def_name) {
                    return self.validate_recursive(data, def_schema, path);
                }
            }
        }

        Err(ConfigError::validation_error(
            format!("Cannot resolve reference '{}' at path '{}'", ref_str, path),
            Some(path.to_string()),
        ))
    }

    /// Get JSON type name
    fn get_json_type(&self, value: &Value) -> String {
        match value {
            Value::Null => "null".to_string(),
            Value::Bool(_) => "boolean".to_string(),
            Value::Number(_) => "number".to_string(),
            Value::String(_) => "string".to_string(),
            Value::Array(_) => "array".to_string(),
            Value::Object(_) => "object".to_string(),
        }
    }

    /// Check if value can be coerced to target types
    fn can_coerce(&self, value: &Value, target_types: &[String]) -> bool {
        match value {
            Value::String(s) => {
                // String to number
                if target_types.contains(&"number".to_string()) {
                    if s.parse::<f64>().is_ok() {
                        return true;
                    }
                }
                // String to boolean
                if target_types.contains(&"boolean".to_string()) {
                    if s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("false") {
                        return true;
                    }
                }
            }
            Value::Number(_) => {
                // Number to string
                if target_types.contains(&"string".to_string()) {
                    return true;
                }
            }
            Value::Bool(_) => {
                // Boolean to string
                if target_types.contains(&"string".to_string()) {
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
        !s.is_empty() && s.len() <= 253 && s.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-')
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