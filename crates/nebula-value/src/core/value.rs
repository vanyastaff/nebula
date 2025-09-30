use crate::core::error::{ValueResult, ValueErrorExt};
use crate::core::NebulaError;
use crate::core::kind::ValueKind;
use crate::types::*;
use std::collections::HashMap;
use std::fmt;

#[derive(Clone, Default)]
#[non_exhaustive]
pub enum Value {
    #[default]
    Null,
    Bool(Boolean),
    Number(Number),
    String(Text),
    Array(Array),
    Object(Object),
    Bytes(crate::types::Bytes),
    Date(Date),
    Time(Time),
    DateTime(DateTime),
    Duration(Duration),
    File(File),
}

impl Value {
    // ==================== Constructors ====================

    /// Create a null value
    #[inline]
    pub const fn null() -> Self {
        Value::Null
    }

    /// Create a boolean value
    #[inline]
    pub fn bool(v: bool) -> Self {
        Value::Bool(Boolean::new(v))
    }

    /// Create an integer value
    #[inline]
    pub fn int(v: i64) -> Self {
        Value::Number(Number::int(v))
    }

    /// Create a float value
    #[inline]
    pub fn float(v: f64) -> Self {
        Value::Number(Number::float(v))
    }

    /// Create a string value
    #[inline]
    pub fn string<S: Into<String>>(v: S) -> Self {
        Value::String(Text::new(v.into()))
    }

    /// Create an array value
    #[inline]
    pub fn array(v: Vec<Value>) -> Self {
        Value::Array(Array::new(v))
    }

    /// Create an object value
    #[inline]
    pub fn object(v: HashMap<String, Value>) -> Self {
        let pairs = v.into_iter();
        Value::Object(Object::from_pairs(pairs))
    }

    /// Create a decimal value
    #[inline]
    pub fn decimal(v: rust_decimal::Decimal) -> Self {
        Value::Number(Number::decimal(v))
    }

    /// Create a bytes value
    #[inline]
    pub fn bytes<B: Into<Vec<u8>>>(v: B) -> Self {
        Value::Bytes(crate::types::Bytes::from(v.into()))
    }

    /// Create a file value
    #[inline]
    pub fn file(v: File) -> Self {
        Value::File(v)
    }

    /// Create a date value
    pub fn date(year: i32, month: u32, day: u32) -> Result<Self, crate::types::DateError> {
        Ok(Value::Date(crate::types::Date::new(year, month, day)?))
    }

    /// Create a time value
    pub fn time(hour: u32, minute: u32, second: u32) -> Result<Self, crate::types::TimeError> {
        Ok(Value::Time(crate::types::Time::new(hour, minute, second)?))
    }

    /// Create a datetime value
    pub fn datetime(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> Result<Self, crate::types::DateTimeError> {
        Ok(Value::DateTime(crate::types::DateTime::new(year, month, day, hour, minute, second)?))
    }

    /// Create a duration value from seconds
    ///
    /// # Panics
    /// Panics if seconds is negative
    pub fn duration_seconds(seconds: i64) -> Self {
        match seconds.try_into() {
            Ok(secs) => Value::Duration(crate::types::Duration::from_secs(secs)),
            Err(_) => panic!("Duration seconds must be non-negative, got: {}", seconds),
        }
    }

    /// Create a duration value from milliseconds
    ///
    /// # Panics
    /// Panics if milliseconds is negative
    pub fn duration_millis(millis: i64) -> Self {
        match millis.try_into() {
            Ok(ms) => Value::Duration(crate::types::Duration::from_millis(ms)),
            Err(_) => panic!("Duration milliseconds must be non-negative, got: {}", millis),
        }
    }

    /// Try to create a duration value from seconds
    ///
    /// Returns None if seconds is negative
    pub fn try_duration_seconds(seconds: i64) -> Option<Self> {
        seconds.try_into()
            .ok()
            .map(|secs| Value::Duration(crate::types::Duration::from_secs(secs)))
    }

    /// Try to create a duration value from milliseconds
    ///
    /// Returns None if milliseconds is negative
    pub fn try_duration_millis(millis: i64) -> Option<Self> {
        millis.try_into()
            .ok()
            .map(|ms| Value::Duration(crate::types::Duration::from_millis(ms)))
    }

    // ==================== Builder Methods ====================

    /// Create an empty array
    pub fn empty_array() -> Self {
        Value::Array(Array::new(Vec::new()))
    }

    /// Create an empty object
    pub fn empty_object() -> Self {
        Value::Object(Object::new())
    }

    /// Create an array from an iterator
    pub fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = Value>,
    {
        Value::Array(Array::from_iter(iter))
    }

    /// Create an object from key-value pairs
    pub fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<Value>,
    {
        let obj = Object::new();
        let final_obj = pairs.into_iter().fold(obj, |mut acc, (k, v)| {
            let _ = acc.insert(k.into(), v.into());
            acc
        });
        Value::Object(final_obj)
    }

    // ==================== Convenience Constructors ====================

    /// Create a string value from &str
    pub fn str(s: &str) -> Self {
        Value::string(s)
    }

    /// Create a float from integer
    pub fn float_from_int(v: i64) -> Self {
        Value::float(v as f64)
    }

    /// Create date from ISO string (YYYY-MM-DD)
    pub fn date_from_iso(s: &str) -> Result<Self, crate::types::DateError> {
        Ok(Value::Date(crate::types::Date::parse_iso(s)?))
    }

    /// Create time from ISO string (HH:MM:SS)
    pub fn time_from_iso(s: &str) -> Result<Self, crate::types::TimeError> {
        Ok(Value::Time(crate::types::Time::parse_iso(s)?))
    }

    /// Create datetime from ISO string
    pub fn datetime_from_iso(s: &str) -> Result<Self, crate::types::DateTimeError> {
        Ok(Value::DateTime(crate::types::DateTime::parse_iso(s)?))
    }

    /// Create bytes from base64 string
    pub fn bytes_from_base64(s: &str) -> Result<Self, crate::types::BytesError> {
        Ok(Value::Bytes(crate::types::Bytes::from_base64(s)?))
    }

    /// Create bytes from hex string
    pub fn bytes_from_hex(s: &str) -> Result<Self, crate::types::BytesError> {
        Ok(Value::Bytes(crate::types::Bytes::from_hex(s)?))
    }

    // ==================== Type Checking ====================

    /// Check if value is null
    #[inline]
    pub const fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Check if value is boolean
    #[inline]
    pub const fn is_bool(&self) -> bool {
        matches!(self, Value::Bool(_))
    }

    /// Check if value is integer
    #[inline]
    pub const fn is_int(&self) -> bool {
        matches!(self, Value::Number(Number::Int(_)))
    }

    /// Check if value is float
    #[inline]
    pub const fn is_float(&self) -> bool {
        matches!(self, Value::Number(Number::Float(_)))
    }

    /// Check if value is decimal
    #[inline]
    pub const fn is_decimal(&self) -> bool {
        matches!(self, Value::Number(Number::Decimal(_)))
    }

    /// Check if value is any numeric type
    #[inline]
    pub const fn is_numeric(&self) -> bool {
        matches!(self, Value::Number(_))
    }

    /// Check if value is string
    #[inline]
    pub const fn is_string(&self) -> bool {
        matches!(self, Value::String(_))
    }

    /// Check if value is array
    #[inline]
    pub const fn is_array(&self) -> bool {
        matches!(self, Value::Array(_))
    }

    /// Check if value is object
    #[inline]
    pub const fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }

    /// Check if value is a collection (array or object)
    #[inline]
    pub const fn is_collection(&self) -> bool {
        matches!(self, Value::Array(_) | Value::Object(_))
    }

    /// Check if value is file
    #[inline]
    pub const fn is_file(&self) -> bool {
        matches!(self, Value::File(_))
    }

    /// Get the type name as string
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(Number::Int(_)) => "integer",
            Value::Number(Number::Float(_)) => "float",
            Value::Number(Number::Decimal(_)) => "decimal",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
            Value::Bytes(_) => "bytes",
            Value::Date(_) => "date",
            Value::Time(_) => "time",
            Value::DateTime(_) => "datetime",
            Value::Duration(_) => "duration",
            Value::File(_) => "file",
        }
    }

    // ==================== Safe Accessors ====================

    /// Try to get boolean value
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(b.value()),
            _ => None,
        }
    }

    /// Try to get integer value
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Number(n) => n.to_i64(),
            _ => None,
        }
    }

    /// Alias used in tests/utilities
    pub fn as_i64(&self) -> Option<i64> {
        self.as_int()
    }

    /// Try to get float value
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(n.to_f64()),
            _ => None,
        }
    }

    /// Try to get decimal value
    pub fn as_decimal(&self) -> Option<&rust_decimal::Decimal> {
        match self {
            Value::Number(Number::Decimal(d)) => Some(d),
            _ => None,
        }
    }

    /// Try to get mutable decimal value
    pub fn as_decimal_mut(&mut self) -> Option<&mut rust_decimal::Decimal> {
        match self {
            Value::Number(Number::Decimal(d)) => Some(d),
            _ => None,
        }
    }

    /// Try to get string reference
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Try to get array reference
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    /// Try to get mutable array reference (not supported for persistent Array)
    pub fn as_array_mut(&mut self) -> Option<&mut [Value]> {
        None
    }

    /// Try to get object reference
    pub fn as_object(&self) -> Option<&Object> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    /// Try to get mutable object reference (note: Object is persistent; inner map is not mutable)
    pub fn as_object_mut(&mut self) -> Option<&mut Object> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    /// Try to get file reference
    pub fn as_file(&self) -> Option<&File> {
        match self {
            Value::File(f) => Some(f),
            _ => None,
        }
    }

    /// Try to get mutable file reference
    pub fn as_file_mut(&mut self) -> Option<&mut File> {
        match self {
            Value::File(f) => Some(f),
            _ => None,
        }
    }

    // ==================== Type Information ====================

    /// Get the kind/type of this value
    pub fn kind(&self) -> ValueKind {
        ValueKind::from_value(self)
    }

    // ==================== Safe Getters (try_*) ====================

    /// Try to get boolean value with error
    pub fn try_as_bool(&self) -> ValueResult<bool> {
        self.as_bool()
            .ok_or_else(|| NebulaError::value_type_mismatch("boolean", self.kind().to_string()))
    }

    /// Try to get integer value with error
    pub fn try_as_int(&self) -> ValueResult<i64> {
        self.as_int()
            .ok_or_else(|| NebulaError::value_type_mismatch("integer", self.kind().to_string()))
    }

    /// Try to get float value with error
    pub fn try_as_float(&self) -> ValueResult<f64> {
        self.as_float()
            .ok_or_else(|| NebulaError::value_type_mismatch("float", self.kind().to_string()))
    }

    /// Try to get string value with error
    pub fn try_as_str(&self) -> ValueResult<&str> {
        self.as_str()
            .ok_or_else(|| NebulaError::value_type_mismatch("string", self.kind().to_string()))
    }

    /// Try to get array reference with error
    pub fn try_as_array(&self) -> ValueResult<&[Value]> {
        self.as_array()
            .ok_or_else(|| NebulaError::value_type_mismatch("array", self.kind().to_string()))
    }

    /// Try to get object reference with error
    pub fn try_as_object(&self) -> ValueResult<&Object> {
        self.as_object()
            .ok_or_else(|| NebulaError::value_type_mismatch("object", self.kind().to_string()))
    }

    /// Try to get decimal reference with error
    pub fn try_as_decimal(&self) -> ValueResult<&rust_decimal::Decimal> {
        self.as_decimal()
            .ok_or_else(|| NebulaError::value_type_mismatch("decimal", self.kind().to_string()))
    }

    /// Try to get file reference with error
    pub fn try_as_file(&self) -> ValueResult<&File> {
        self.as_file()
            .ok_or_else(|| NebulaError::value_type_mismatch("file", self.kind().to_string()))
    }

    /// Try to get value by path with error
    pub fn try_get_path(&self, path: &str) -> ValueResult<&Value> {
        self.get_path(path)
            .ok_or_else(|| NebulaError::value_path_not_found(path))
    }

    // ==================== Collection Methods ====================

    /// Get the length/size of the value
    ///
    /// Returns the number of elements for arrays and objects,
    /// number of bytes for bytes, number of characters for strings,
    /// and 1 for other types (0 for null)
    pub fn len(&self) -> usize {
        match self {
            Value::Null => 0,
            Value::Array(a) => a.len(),
            Value::Object(o) => o.len(),
            Value::String(s) => s.len(),
            Value::Bytes(b) => b.len(),
            _ => 1,
        }
    }

    /// Check if the value is empty
    ///
    /// Returns true for null, empty arrays, empty objects,
    /// empty strings, and empty bytes
    pub fn is_empty(&self) -> bool {
        match self {
            Value::Null => true,
            Value::Array(a) => a.is_empty(),
            Value::Object(o) => o.is_empty(),
            Value::String(s) => s.is_empty(),
            Value::Bytes(b) => b.is_empty(),
            _ => false,
        }
    }

    /// Get array element by index
    pub fn get_index(&self, index: usize) -> Option<&Value> {
        match self {
            Value::Array(a) => a.get(index),
            _ => None,
        }
    }

    /// Get object value by key
    pub fn get_key(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(o) => o.get(key),
            _ => None,
        }
    }

    /// Check if object contains key
    pub fn contains_key(&self, key: &str) -> bool {
        match self {
            Value::Object(o) => o.contains_key(key),
            _ => false,
        }
    }

    /// Get all keys from object
    pub fn keys(&self) -> Vec<String> {
        match self {
            Value::Object(o) => o.keys(),
            _ => Vec::new(),
        }
    }

    /// Get all values from object or array as owned values
    pub fn values(&self) -> Vec<Value> {
        match self {
            Value::Object(o) => o.values(),
            Value::Array(a) => a.iter().cloned().collect(),
            _ => Vec::new(),
        }
    }

    /// Get all values from array as references
    pub fn values_ref(&self) -> Vec<&Value> {
        match self {
            Value::Array(a) => a.iter().collect(),
            _ => Vec::new(),
        }
    }

    /// Get key-value pairs from object
    pub fn entries(&self) -> Vec<(String, &Value)> {
        match self {
            Value::Object(o) => o.iter().map(|(k, v)| (k.clone(), v)).collect(),
            _ => Vec::new(),
        }
    }

    // ==================== Path Access ====================

    /// Get value by path (e.g., "user.address.city")
    pub fn get_path(&self, path: &str) -> Option<&Value> {
        let parts: Vec<&str> = path.split('.').collect();
        self.get_path_segments(&parts)
    }

    /// Get value by path segments
    pub fn get_path_segments(&self, segments: &[&str]) -> Option<&Value> {
        if segments.is_empty() {
            return Some(self);
        }

        let (first, rest) = segments.split_first()?;

        match self {
            Value::Object(obj) => {
                let next = obj.get(first)?;
                if rest.is_empty() {
                    Some(next)
                } else {
                    next.get_path_segments(rest)
                }
            }
            Value::Array(arr) => {
                let index = first.parse::<usize>().ok()?;
                let next = arr.get(index)?;
                if rest.is_empty() {
                    Some(next)
                } else {
                    next.get_path_segments(rest)
                }
            }
            _ => None,
        }
    }

    /// Set value by path
    pub fn set_path(&mut self, path: &str, value: Value) -> ValueResult<()> {
        let parts: Vec<&str> = path.split('.').collect();
        self.set_path_segments(&parts, value)
    }

    /// Set value by path segments
    pub fn set_path_segments(&mut self, segments: &[&str], value: Value) -> ValueResult<()> {
        if segments.is_empty() {
            *self = value;
            return Ok(());
        }

        let (first, rest) = segments
            .split_first()
            .ok_or_else(|| NebulaError::internal("Empty path segments"))?;

        if rest.is_empty() {
            // Last segment - set the value
            match self {
                Value::Object(obj) => {
                    let new_obj = obj.insert(first.to_string(), value);
                    *obj = new_obj;
                    Ok(())
                }
                Value::Array(arr) => {
                    let index = first
                        .parse::<usize>()
                        .map_err(|_| NebulaError::value_parse_error("array index", *first))?;
                    if index >= arr.len() {
                        return Err(NebulaError::value_index_out_of_bounds(index, arr.len()));
                    }
                    let new_arr = arr
                        .set(index, value)
                        .map_err(|e| NebulaError::internal(format!("array set error: {:?}", e)))?;
                    *arr = new_arr;
                    Ok(())
                }
                _ => Err(NebulaError::value_operation_not_supported(
                    "set_path",
                    self.type_name(),
                )),
            }
        } else {
            // Navigate deeper
            match self {
                Value::Object(obj) => {
                    let mut next_val = obj
                        .get(first)
                        .cloned()
                        .unwrap_or(Value::Object(Object::new()));
                    next_val.set_path_segments(rest, value)?;
                    let new_obj = obj.insert(first.to_string(), next_val);
                    *obj = new_obj;
                    Ok(())
                }
                Value::Array(arr) => {
                    let index = first
                        .parse::<usize>()
                        .map_err(|_| NebulaError::value_parse_error("array index", *first))?;
                    if index >= arr.len() {
                        return Err(NebulaError::value_index_out_of_bounds(index, arr.len()));
                    }
                    let mut elem = arr
                        .get(index)
                        .cloned()
                        .ok_or_else(|| NebulaError::value_index_out_of_bounds(index, arr.len()))?;
                    elem.set_path_segments(rest, value)?;
                    let new_arr = arr
                        .set(index, elem)
                        .map_err(|e| NebulaError::internal(format!("array set error: {:?}", e)))?;
                    *arr = new_arr;
                    Ok(())
                }
                _ => Err(NebulaError::value_operation_not_supported(
                    "set_path",
                    self.type_name(),
                )),
            }
        }
    }

    /// Insert value by path, creating intermediate objects as needed
    pub fn insert_path(&mut self, path: &str, value: Value) -> ValueResult<()> {
        let parts: Vec<&str> = path.split('.').collect();
        self.insert_path_segments(&parts, value)
    }

    /// Insert value by path segments, creating intermediate objects as needed
    pub fn insert_path_segments(&mut self, segments: &[&str], value: Value) -> ValueResult<()> {
        if segments.is_empty() {
            *self = value;
            return Ok(());
        }

        let (first, rest) = segments
            .split_first()
            .ok_or_else(|| NebulaError::internal("Empty path segments"))?;

        if rest.is_empty() {
            // Last segment - set the value
            match self {
                Value::Object(obj) => {
                    let new_obj = obj.insert(first.to_string(), value);
                    *obj = new_obj;
                    Ok(())
                }
                Value::Array(arr) => {
                    let index = first
                        .parse::<usize>()
                        .map_err(|_| NebulaError::value_parse_error("array index", *first))?;
                    if index >= arr.len() {
                        return Err(NebulaError::value_index_out_of_bounds(index, arr.len()));
                    }
                    let new_arr = arr
                        .set(index, value)
                        .map_err(|e| NebulaError::internal(format!("array set error: {:?}", e)))?;
                    *arr = new_arr;
                    Ok(())
                }
                Value::Null => {
                    // Create object structure
                    *self = Value::object(HashMap::new());
                    self.insert_path_segments(segments, value)
                }
                _ => Err(NebulaError::value_operation_not_supported(
                    "insert_path",
                    self.type_name(),
                )),
            }
        } else {
            // Navigate deeper - create intermediate structure if needed
            match self {
                Value::Object(obj) => {
                    let mut next_val = obj
                        .get(first)
                        .cloned()
                        .unwrap_or_else(|| Value::object(HashMap::new())); // Create intermediate object

                    next_val.insert_path_segments(rest, value)?;
                    let new_obj = obj.insert(first.to_string(), next_val);
                    *obj = new_obj;
                    Ok(())
                }
                Value::Array(arr) => {
                    let index = first
                        .parse::<usize>()
                        .map_err(|_| NebulaError::value_parse_error("array index", *first))?;
                    if index >= arr.len() {
                        return Err(NebulaError::value_index_out_of_bounds(index, arr.len()));
                    }
                    let mut next_val = arr.get(index).cloned().unwrap_or_else(|| Value::object(HashMap::new()));
                    next_val.insert_path_segments(rest, value)?;
                    let new_arr = arr
                        .set(index, next_val)
                        .map_err(|e| NebulaError::internal(format!("array set error: {:?}", e)))?;
                    *arr = new_arr;
                    Ok(())
                }
                Value::Null => {
                    // Create object structure and continue
                    *self = Value::object(HashMap::new());
                    self.insert_path_segments(segments, value)
                }
                _ => Err(NebulaError::value_operation_not_supported(
                    "insert_path",
                    self.type_name(),
                )),
            }
        }
    }

    /// Get value by ValuePath
    pub fn get_by_path(&self, path: &crate::core::path::ValuePath) -> Option<&Value> {
        let mut segments = Vec::new();

        for segment in path.segments() {
            match segment {
                crate::core::path::PathSegment::Key(k) => segments.push(k.as_str()),
                crate::core::path::PathSegment::Index(i) => {
                    // We need to handle index segments differently since we need &str
                    // For now, we'll convert to path notation and parse again
                    return None; // Limitation: can't easily handle mixed types here
                },
                _ => return None, // Wildcards not supported in simple get
            }
        }

        self.get_path_segments(&segments)
    }

    /// Set value by ValuePath
    pub fn set_by_path(&mut self, path: &crate::core::path::ValuePath, value: Value) -> ValueResult<()> {
        let mut segments = Vec::new();

        for segment in path.segments() {
            match segment {
                crate::core::path::PathSegment::Key(k) => segments.push(k.clone()),
                crate::core::path::PathSegment::Index(i) => segments.push(i.to_string()),
                _ => return Err(NebulaError::validation("Wildcards not supported in set_by_path".to_string())),
            }
        }

        let str_segments: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
        self.set_path_segments(&str_segments, value)
    }

    // ==================== Conversions ====================

    /// Try to coerce value to string
    pub fn coerce_to_string(&self) -> String {
        match self {
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.to_string(),
            Value::Array(a) => format!("[array of {} items]", a.len()),
            Value::Object(o) => format!("[object with {} keys]", o.len()),
            Value::Bytes(b) => format!("[{} bytes]", b.len()),
            Value::Date(d) => d.to_string(),
            Value::Time(t) => t.to_string(),
            Value::DateTime(dt) => dt.to_string(),
            Value::Duration(d) => d.to_string(),
            Value::File(f) => f.to_string(),
        }
    }

    /// Try to coerce value to boolean
    pub fn coerce_to_bool(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => b.value(),
            Value::Number(n) => !n.is_zero() && n.is_finite(),
            Value::String(s) => !s.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => !o.is_empty(),
            Value::Bytes(b) => !b.is_empty(),
            Value::Date(_) | Value::Time(_) | Value::DateTime(_) | Value::Duration(_) | Value::File(_) => true,
        }
    }

    /// Try to coerce value to number
    pub fn coerce_to_number(&self) -> Option<f64> {
        match self {
            Value::Bool(b) => Some(if b.value() { 1.0 } else { 0.0 }),
            Value::Number(n) => Some(n.to_f64()),
            Value::String(s) => s.as_str().parse().ok(),
            _ => None,
        }
    }

    // ==================== Validation Methods ====================

    /// Check if value is a finite number (not NaN or infinite)
    pub fn is_finite_number(&self) -> bool {
        match self {
            Value::Number(n) => n.is_finite(),
            _ => false,
        }
    }

    /// Check if value is a positive number
    pub fn is_positive(&self) -> bool {
        match self {
            Value::Number(n) => n.is_positive(),
            _ => false,
        }
    }

    /// Check if value is a negative number
    pub fn is_negative(&self) -> bool {
        match self {
            Value::Number(n) => n.is_negative(),
            _ => false,
        }
    }

    /// Check if value is zero
    pub fn is_zero(&self) -> bool {
        match self {
            Value::Number(n) => n.is_zero(),
            _ => false,
        }
    }

    /// Check if value represents a "truthy" value
    ///
    /// Similar to JavaScript truthiness:
    /// - null, false, 0, "", [], {} are falsy
    /// - everything else is truthy
    pub fn is_truthy(&self) -> bool {
        self.coerce_to_bool()
    }

    /// Check if value represents a "falsy" value
    pub fn is_falsy(&self) -> bool {
        !self.is_truthy()
    }

    /// Validate that value matches expected type
    pub fn validate_type(&self, expected: ValueKind) -> ValueResult<()> {
        let actual = self.kind();
        if actual == expected {
            Ok(())
        } else {
            Err(NebulaError::value_type_mismatch(
                expected.to_string(),
                actual.to_string(),
            ))
        }
    }

    /// Validate that numeric value is within range
    pub fn validate_range(&self, min: f64, max: f64) -> ValueResult<()> {
        match self.coerce_to_number() {
            Some(n) if n >= min && n <= max => Ok(()),
            Some(n) => Err(NebulaError::validation(format!(
                "Value {} is outside allowed range [{}, {}]",
                n, min, max
            ))),
            None => Err(NebulaError::value_type_mismatch(
                "number",
                self.kind().to_string(),
            )),
        }
    }

    /// Validate that array/object has expected length
    pub fn validate_length(&self, min: Option<usize>, max: Option<usize>) -> ValueResult<()> {
        let len = self.len();

        if let Some(min_len) = min {
            if len < min_len {
                return Err(NebulaError::validation(format!(
                    "Length {} is less than minimum {}",
                    len, min_len
                )));
            }
        }

        if let Some(max_len) = max {
            if len > max_len {
                return Err(NebulaError::validation(format!(
                    "Length {} is greater than maximum {}",
                    len, max_len
                )));
            }
        }

        Ok(())
    }

    // ==================== Utility Methods ====================

    /// Create a deep clone of this value
    ///
    /// This is equivalent to `clone()` but more explicit about the cost
    pub fn deep_clone(&self) -> Self {
        self.clone()
    }

    /// Check if two values are deeply equal
    ///
    /// This is more thorough than `==` and handles special float values
    pub fn deep_equals(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Number(Number::Float(a)), Value::Number(Number::Float(b))) => {
                if a.is_nan() && b.is_nan() {
                    true
                } else {
                    a == b
                }
            }
            _ => self == other,
        }
    }

    // ==================== Additional Collection Operations ====================

    /// Get value from object by key (alias for get_key)
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.get_key(key)
    }

    /// Check if array contains a specific value
    pub fn contains(&self, value: &Value) -> bool {
        match self {
            Value::Array(arr) => arr.iter().any(|v| v == value),
            _ => false,
        }
    }

    /// Merge two values (for objects, concatenate for arrays)
    pub fn merge(&mut self, other: Value) -> ValueResult<()> {
        match (self, other) {
            (Value::Object(o1), Value::Object(o2)) => {
                let merged = o1.merge(&o2);
                *o1 = merged;
                Ok(())
            }
            (Value::Array(a1), Value::Array(a2)) => {
                let concatenated = a1.concat(&a2);
                *a1 = concatenated;
                Ok(())
            }
            (s, o) => Err(NebulaError::value_type_mismatch(s.type_name(), o.type_name())),
        }
    }
}

// ==================== Display Implementation ====================

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_display(f, 0, false)
    }
}

impl Value {
    /// Internal formatter with indentation support
    fn fmt_display(&self, f: &mut fmt::Formatter<'_>, indent: usize, pretty: bool) -> fmt::Result {
        match self {
            Value::Null => write!(f, "null"),
            Value::Bool(b) => write!(f, "{}", b.value()),
            Value::Number(n) => write!(f, "{}", n),
            Value::String(s) => {
                if f.alternate() {
                    write!(f, "\"{}\"", s.as_str().replace('\"', "\\\""))
                } else {
                    write!(f, "{}", s.as_str())
                }
            }
            Value::Array(a) => {
                if a.is_empty() {
                    return write!(f, "[]");
                }

                if pretty && a.len() > 3 {
                    writeln!(f, "[")?;
                    for (i, item) in a.iter().enumerate() {
                        if i > 0 {
                            writeln!(f, ",")?;
                        }
                        write!(f, "{}", "  ".repeat(indent + 1))?;
                        item.fmt_display(f, indent + 1, true)?;
                    }
                    writeln!(f)?;
                    write!(f, "{}]", "  ".repeat(indent))
                } else {
                    write!(f, "[")?;
                    for (i, item) in a.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        item.fmt_display(f, indent, false)?;
                    }
                    write!(f, "]")
                }
            }
            Value::Object(o) => {
                if o.is_empty() {
                    return write!(f, "{{}}");
                }

                if pretty && o.len() > 2 {
                    writeln!(f, "{{")?;
                    for (i, (k, v)) in o.iter().enumerate() {
                        if i > 0 {
                            writeln!(f, ",")?;
                        }
                        write!(f, "{}\"{}\": ", "  ".repeat(indent + 1), k)?;
                        v.fmt_display(f, indent + 1, true)?;
                    }
                    writeln!(f)?;
                    write!(f, "{}}}", "  ".repeat(indent))
                } else {
                    write!(f, "{{")?;
                    for (i, (k, v)) in o.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "\"{}\": ", k)?;
                        v.fmt_display(f, indent, false)?;
                    }
                    write!(f, "}}")
                }
            }
            Value::Bytes(b) => {
                if b.len() > 32 {
                    write!(f, "<{} bytes>", b.len())
                } else {
                    write!(f, "{}", b)
                }
            }
            Value::Date(d) => write!(f, "{}", d.to_iso_string()),
            Value::Time(t) => write!(f, "{}", t.to_iso_string()),
            Value::DateTime(dt) => write!(f, "{}", dt.to_iso_string()),
            Value::Duration(d) => write!(f, "{}", d.to_human_string()),
            Value::File(file) => write!(f, "{}", file),
        }
    }

    /// Pretty-print the value with proper indentation
    pub fn pretty_print(&self) -> String {
        struct PrettyFormatter<'a>(&'a Value);

        impl<'a> fmt::Display for PrettyFormatter<'a> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt_display(f, 0, true)
            }
        }

        PrettyFormatter(self).to_string()
    }
}

// ==================== Default Implementation ====================

// ==================== PartialEq Implementation ====================

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => a == b,
            (Value::Object(a), Value::Object(b)) => a == b,
            (Value::Bytes(a), Value::Bytes(b)) => a == b,
            (Value::Date(a), Value::Date(b)) => a == b,
            (Value::Time(a), Value::Time(b)) => a == b,
            (Value::DateTime(a), Value::DateTime(b)) => a == b,
            (Value::Duration(a), Value::Duration(b)) => a == b,
            (Value::File(a), Value::File(b)) => a == b,
            _ => false,
        }
    }
}

// ==================== PartialOrd Implementation ====================

impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        use std::cmp::Ordering;

        // Define explicit ordering for each variant
        fn value_order(value: &Value) -> u8 {
            match value {
                Value::Null => 0,
                Value::Bool(_) => 1,
                Value::Number(_) => 2,
                Value::String(_) => 3,
                Value::Array(_) => 4,
                Value::Object(_) => 5,
                Value::Bytes(_) => 6,
                Value::Date(_) => 7,
                Value::Time(_) => 8,
                Value::DateTime(_) => 9,
                Value::Duration(_) => 10,
                Value::File(_) => 11,
            }
        }

        let self_order = value_order(self);
        let other_order = value_order(other);

        match self_order.cmp(&other_order) {
            Ordering::Equal => {
                // Same type, compare values
                match (self, other) {
                    (Value::Null, Value::Null) => Some(Ordering::Equal),
                    (Value::Bool(a), Value::Bool(b)) => a.partial_cmp(b),
                    (Value::Number(a), Value::Number(b)) => a.partial_cmp(b),
                    (Value::String(a), Value::String(b)) => a.partial_cmp(b),
                    (Value::Array(a), Value::Array(b)) => a.partial_cmp(b),
                    (Value::Object(a), Value::Object(b)) => a.partial_cmp(b),
                    (Value::Bytes(a), Value::Bytes(b)) => a.partial_cmp(b),
                    (Value::Date(a), Value::Date(b)) => a.partial_cmp(b),
                    (Value::Time(a), Value::Time(b)) => a.partial_cmp(b),
                    (Value::DateTime(a), Value::DateTime(b)) => a.partial_cmp(b),
                    (Value::Duration(a), Value::Duration(b)) => a.partial_cmp(b),
                    (Value::File(a), Value::File(b)) => a.partial_cmp(b),
                    _ => None,
                }
            },
            ordering => Some(ordering),
        }
    }
}

// ==================== Eq Implementation ====================
// SPECIAL CASE: We implement Eq for Value even though f64 NaN ≠ NaN
// This is required for HashMap/HashSet usage and follows common practice
// in data systems where NaN == NaN for hashing purposes.
// The Hash implementation uses f64::to_bits() which treats equal bit patterns as equal.

impl Eq for Value {}

// ==================== Ord Implementation ====================
// Note: Value cannot implement Ord because Float::partial_cmp returns None for NaN

// ==================== Hash Implementation ====================

impl std::hash::Hash for Value {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hash the discriminant first for different variants
        std::mem::discriminant(self).hash(state);

        // Then hash the actual value
        match self {
            Value::Null => {},
            Value::Bool(b) => b.hash(state),
            Value::Number(n) => n.hash(state),
            Value::String(s) => s.hash(state),
            Value::Array(a) => a.hash(state),
            Value::Object(o) => o.hash(state),
            Value::Bytes(b) => b.hash(state),
            Value::Date(d) => d.hash(state),
            Value::Time(t) => t.hash(state),
            Value::DateTime(dt) => dt.hash(state),
            Value::Duration(dur) => dur.hash(state),
            Value::File(f) => f.hash(state),
        }
    }
}

// ==================== Debug Implementation ====================

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.alternate() {
            // Pretty debug formatting
            self.fmt_debug_pretty(f, 0)
        } else {
            // Compact debug formatting
            self.fmt_debug_compact(f)
        }
    }
}

impl Value {
    /// Compact debug formatting
    fn fmt_debug_compact(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Null => write!(f, "Null"),
            Value::Bool(b) => write!(f, "Bool({})", b.value()),
            Value::Number(Number::Int(i)) => write!(f, "Int({})", i),
            Value::Number(Number::Float(fl)) => {
                if fl.is_finite() {
                    write!(f, "Float({})", fl)
                } else if fl.is_nan() {
                    write!(f, "Float(NaN)")
                } else if fl.is_infinite() && fl.is_sign_positive() {
                    write!(f, "Float(+∞)")
                } else {
                    write!(f, "Float(-∞)")
                }
            }
            Value::Number(Number::Decimal(d)) => write!(f, "Decimal({})", d),
            Value::String(s) => write!(f, "String({:?})", s.as_str()),
            Value::Array(a) => write!(f, "Array[{}]({:?})", a.len(), a.as_slice()),
            Value::Object(o) => write!(f, "Object[{}]({:?})", o.len(), o.as_ref()),
            Value::Bytes(b) => {
                if b.len() <= 16 {
                    write!(f, "Bytes[{}]({:?})", b.len(), b.as_slice())
                } else {
                    write!(f, "Bytes[{}]({:02x?}...)", b.len(), &b.as_slice()[..8])
                }
            }
            Value::Date(d) => write!(f, "Date({})", d.to_iso_string()),
            Value::Time(t) => write!(f, "Time({})", t.to_iso_string()),
            Value::DateTime(dt) => write!(f, "DateTime({})", dt.to_iso_string()),
            Value::Duration(dur) => write!(f, "Duration({})", dur.to_human_string()),
            Value::File(file) => write!(f, "File({:?})", file),
        }
    }

    /// Pretty debug formatting with indentation
    fn fmt_debug_pretty(&self, f: &mut std::fmt::Formatter<'_>, indent: usize) -> std::fmt::Result {
        let indent_str = "  ".repeat(indent);
        let next_indent_str = "  ".repeat(indent + 1);

        match self {
            Value::Null => write!(f, "Value::Null"),
            Value::Bool(b) => write!(f, "Value::Bool({})", b.value()),
            Value::Number(Number::Int(i)) => write!(f, "Value::Number(Int({}))", i),
            Value::Number(Number::Float(fl)) => {
                if fl.is_finite() {
                    write!(f, "Value::Number(Float({}))", fl)
                } else if fl.is_nan() {
                    write!(f, "Value::Number(Float(NaN))")
                } else if fl.is_infinite() && fl.is_sign_positive() {
                    write!(f, "Value::Number(Float(+∞))")
                } else {
                    write!(f, "Value::Number(Float(-∞))")
                }
            }
            Value::Number(Number::Decimal(d)) => write!(f, "Value::Number(Decimal({}))", d),
            Value::String(s) => write!(f, "Value::String({:?})", s.as_str()),
            Value::Array(a) => {
                writeln!(f, "Value::Array[")?;
                for (i, item) in a.iter().enumerate() {
                    write!(f, "{}  [{}]: ", indent_str, i)?;
                    item.fmt_debug_pretty(f, indent + 1)?;
                    if i < a.len() - 1 {
                        writeln!(f, ",")?;
                    } else {
                        writeln!(f)?;
                    }
                }
                write!(f, "{}]", indent_str)
            }
            Value::Object(o) => {
                writeln!(f, "Value::Object {{")?;
                for (i, (k, v)) in o.iter().enumerate() {
                    write!(f, "{}  {:?}: ", next_indent_str, k)?;
                    v.fmt_debug_pretty(f, indent + 1)?;
                    if i < o.len() - 1 {
                        writeln!(f, ",")?;
                    } else {
                        writeln!(f)?;
                    }
                }
                write!(f, "{}}}", indent_str)
            }
            Value::Bytes(b) => {
                if b.len() <= 32 {
                    write!(f, "Value::Bytes[{}]({:02x?})", b.len(), b.as_slice())
                } else {
                    write!(f, "Value::Bytes[{}]({:02x?}...)", b.len(), &b.as_slice()[..16])
                }
            }
            Value::Date(d) => write!(f, "Value::Date({})", d.to_iso_string()),
            Value::Time(t) => write!(f, "Value::Time({})", t.to_iso_string()),
            Value::DateTime(dt) => write!(f, "Value::DateTime({})", dt.to_iso_string()),
            Value::Duration(dur) => write!(f, "Value::Duration({})", dur.to_human_string()),
            Value::File(file) => write!(f, "Value::File({:?})", file),
        }
    }

    /// Returns a debug representation with type information
    pub fn debug_type(&self) -> &'static str {
        match self {
            Value::Null => "Null",
            Value::Bool(_) => "Bool",
            Value::Number(Number::Int(_)) => "Int",
            Value::Number(Number::Float(_)) => "Float",
            Value::Number(Number::Decimal(_)) => "Decimal",
            Value::String(_) => "String",
            Value::Array(_) => "Array",
            Value::Object(_) => "Object",
            Value::Bytes(_) => "Bytes",
            Value::Date(_) => "Date",
            Value::Time(_) => "Time",
            Value::DateTime(_) => "DateTime",
            Value::Duration(_) => "Duration",
            Value::File(_) => "File",
        }
    }

    /// Returns detailed debug information
    pub fn debug_info(&self) -> String {
        match self {
            Value::Null => "null value".to_string(),
            Value::Bool(b) => format!("boolean: {}", b.value()),
            Value::Number(Number::Int(i)) => format!("integer: {}", i),
            Value::Number(Number::Float(f)) => {
                if f.is_finite() {
                    format!("float: {}", f)
                } else if f.is_nan() {
                    "float: NaN".to_string()
                } else if f.is_infinite() {
                    format!("float: {}∞", if f.is_sign_positive() { "+" } else { "-" })
                } else {
                    format!("float: {}", f)
                }
            }
            Value::Number(Number::Decimal(d)) => format!("decimal: {}", d),
            Value::String(s) => format!("string: {:?} (len: {})", s.as_str(), s.len()),
            Value::Array(a) => format!("array: {} items", a.len()),
            Value::Object(o) => format!("object: {} keys", o.len()),
            Value::Bytes(b) => format!("bytes: {} bytes", b.len()),
            Value::Date(d) => format!("date: {}", d.to_iso_string()),
            Value::Time(t) => format!("time: {}", t.to_iso_string()),
            Value::DateTime(dt) => format!("datetime: {}", dt.to_iso_string()),
            Value::Duration(dur) => format!("duration: {}", dur.to_human_string()),
            Value::File(f) => format!("file: {}", f),
        }
    }
}

// ==================== Serde Implementations ====================

#[cfg(feature = "serde")]
impl serde::Serialize for Value {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Value::Null => serializer.serialize_none(),
            Value::Bool(b) => b.serialize(serializer),
            Value::Number(n) => {
                match n {
                    Number::Int(i) => i.serialize(serializer),
                    Number::Float(f) => {
                        // Handle special float values according to specification
                        if f.is_finite() {
                            f.serialize(serializer)
                        } else if f.is_nan() {
                            serializer.serialize_str("NaN")
                        } else if f.is_infinite() && f.is_sign_positive() {
                            serializer.serialize_str("+Infinity")
                        } else {
                            serializer.serialize_str("-Infinity")
                        }
                    },
                    Number::Decimal(d) => serializer.serialize_str(&d.to_string()),
                }
            },
            Value::String(s) => s.serialize(serializer),
            Value::Array(a) => a.serialize(serializer),
            Value::Object(o) => o.serialize(serializer),
            Value::Bytes(b) => {
                // Bytes should serialize as base64 string
                serializer.serialize_str(&b.to_base64())
            },
            Value::Date(d) => {
                // Date should serialize as ISO 8601 string
                serializer.serialize_str(&d.to_string())
            },
            Value::Time(t) => {
                // Time should serialize as ISO 8601 string
                serializer.serialize_str(&t.to_string())
            },
            Value::DateTime(dt) => {
                // DateTime should serialize as RFC 3339 with Z suffix
                serializer.serialize_str(&dt.to_rfc3339())
            },
            Value::Duration(dur) => {
                // Duration should serialize as human-readable string
                serializer.serialize_str(&dur.to_string())
            },
            Value::File(f) => f.serialize(serializer),
        }
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Value {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Use serde_json::Value as intermediate for flexibility
        let json_value = serde_json::Value::deserialize(deserializer)?;
        Ok(Value::from(json_value))
    }
}

// ==================== From Implementations ====================

impl From<()> for Value {
    fn from(_: ()) -> Self {
        Value::Null
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::bool(v)
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::int(v as i64)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::int(v)
    }
}

impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::float(v as f64)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::float(v)
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::string(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::string(v)
    }
}

impl<T> From<Vec<T>> for Value
where
    T: Into<Value>,
{
    fn from(v: Vec<T>) -> Self {
        Value::Array(Array::new(v.into_iter().map(Into::into).collect()))
    }
}

impl<K, V> From<HashMap<K, V>> for Value
where
    K: Into<String>,
    V: Into<Value>,
{
    fn from(v: HashMap<K, V>) -> Self {
        let pairs = v.into_iter().map(|(k, v)| (k.into(), v.into()));
        Value::Object(Object::from_pairs(pairs))
    }
}

impl From<Vec<u8>> for Value {
    fn from(v: Vec<u8>) -> Self {
        Value::Bytes(Bytes::new(v))
    }
}

impl From<&[u8]> for Value {
    fn from(v: &[u8]) -> Self {
        Value::Bytes(Bytes::copy_from_slice(v))
    }
}

impl From<Option<Value>> for Value {
    fn from(v: Option<Value>) -> Self {
        v.unwrap_or(Value::Null)
    }
}

#[cfg(feature = "serde")]
impl From<serde_json::Value> for Value {
    fn from(v: serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::bool(b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Value::int(i)
                } else if let Some(f) = n.as_f64() {
                    Value::float(f)
                } else {
                    Value::Null
                }
            }
            serde_json::Value::String(s) => {
                // Check for special float values first
                match s.as_str() {
                    "NaN" => Value::float(f64::NAN),
                    "+Infinity" => Value::float(f64::INFINITY),
                    "-Infinity" => Value::float(f64::NEG_INFINITY),
                    _ => {
                        // Try to parse as Decimal for high precision numbers
                        if let Ok(decimal_val) = rust_decimal::Decimal::from_str_exact(&s) {
                            Value::decimal(decimal_val)
                        } else {
                            Value::string(s)
                        }
                    }
                }
            },
            serde_json::Value::Array(arr) => {
                let items = arr.into_iter().map(Value::from).collect::<Vec<_>>();
                Value::Array(Array::new(items))
            }
            serde_json::Value::Object(obj) => {
                let mut map = std::collections::HashMap::with_capacity(obj.len());
                for (k, v) in obj {
                    map.insert(k, Value::from(v));
                }
                Value::object(map)
            }
        }
    }
}

#[cfg(feature = "serde")]
impl From<Value> for serde_json::Value {
    fn from(v: Value) -> Self {
        match v {
            Value::Null => serde_json::Value::Null,
            Value::Bool(b) => serde_json::Value::Bool(b.value()),
            Value::Number(n) => {
                match n.json_strategy() {
                    crate::types::JsonNumberStrategy::Number => {
                        match n {
                            Number::Int(i) => serde_json::Value::Number(serde_json::Number::from(i)),
                            Number::Float(f) => {
                                serde_json::Number::from_f64(f)
                                    .map(serde_json::Value::Number)
                                    .unwrap_or(serde_json::Value::Null)
                            }
                            Number::Decimal(d) => {
                                // For JSON numbers, try to convert to f64 if possible
                                let f = d.try_into().unwrap_or(f64::NAN);
                                if f.is_finite() {
                                    serde_json::Number::from_f64(f)
                                        .map(serde_json::Value::Number)
                                        .unwrap_or(serde_json::Value::String(d.to_string()))
                                } else {
                                    serde_json::Value::String(d.to_string())
                                }
                            }
                        }
                    }
                    crate::types::JsonNumberStrategy::String => {
                        serde_json::Value::String(n.to_string())
                    }
                }
            },
            Value::String(s) => serde_json::Value::String(s.to_string()),
            Value::Array(a) => {
                let vec = a
                    .iter()
                    .cloned()
                    .map(serde_json::Value::from)
                    .collect::<Vec<_>>();
                serde_json::Value::Array(vec)
            }
            Value::Object(o) => {
                let mut map = serde_json::Map::with_capacity(o.len());
                for (k, v) in o.iter() {
                    map.insert(k.clone(), serde_json::Value::from(v.clone()));
                }
                serde_json::Value::Object(map)
            }
            Value::Bytes(b) => serde_json::Value::from(b.clone()),
            Value::Date(d) => serde_json::Value::String(d.to_string()),
            Value::Time(t) => serde_json::Value::String(t.to_string()),
            Value::DateTime(dt) => serde_json::Value::String(dt.to_rfc3339()),
            Value::Duration(dur) => serde_json::Value::String(dur.to_string()),
            Value::File(f) => serde_json::Value::String(format!("<file: {}>", f.metadata().filename.as_deref().unwrap_or("unknown"))),
        }
    }
}


// ==================== TryFrom Implementations ====================

impl TryFrom<Value> for bool {
    type Error = NebulaError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Bool(b) => Ok(b.value()),
            _ => Err(NebulaError::value_conversion_error("bool", &format!("{:?}", value))),
        }
    }
}

impl TryFrom<Value> for i64 {
    type Error = NebulaError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Number(n) => {
                let error_msg = format!("{:?}", n);
                n.to_i64().ok_or_else(||
                    NebulaError::value_conversion_error("i64", &error_msg)
                )
            },
            _ => {
                let error_msg = format!("{:?}", value);
                Err(NebulaError::value_conversion_error("i64", &error_msg))
            },
        }
    }
}

impl TryFrom<Value> for f64 {
    type Error = NebulaError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Number(n) => Ok(n.to_f64()),
            _ => Err(NebulaError::value_conversion_error("f64", &format!("{:?}", value))),
        }
    }
}

impl TryFrom<Value> for String {
    type Error = NebulaError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::String(s) => Ok(s.to_string()),
            _ => Err(NebulaError::value_conversion_error("String", &format!("{:?}", value))),
        }
    }
}

impl TryFrom<Value> for Vec<u8> {
    type Error = NebulaError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Bytes(b) => Ok(b.to_vec()),
            _ => Err(NebulaError::value_conversion_error("Vec<u8>", &format!("{:?}", value))),
        }
    }
}

impl TryFrom<Value> for Vec<Value> {
    type Error = NebulaError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Array(a) => Ok(a.iter().cloned().collect()),
            _ => Err(NebulaError::value_conversion_error("Vec<Value>", &format!("{:?}", value))),
        }
    }
}

impl TryFrom<Value> for std::collections::HashMap<String, Value> {
    type Error = NebulaError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Object(o) => {
                let mut map = std::collections::HashMap::with_capacity(o.len());
                for (k, v) in o.iter() {
                    map.insert(k.clone(), v.clone());
                }
                Ok(map)
            }
            _ => Err(NebulaError::value_conversion_error("HashMap<String, Value>", &format!("{:?}", value))),
        }
    }
}
