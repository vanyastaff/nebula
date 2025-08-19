use std::collections::HashMap;
use std::fmt;
use crate::types::*;
use crate::error::ValueError;

#[derive(Debug, Clone, PartialOrd)]
#[non_exhaustive]
pub enum Value {
    Null,
    Bool(Boolean),
    Int(Integer),
    Float(Float),
    String(Text),
    Array(Array),
    Object(Object),
    Bytes(Bytes),
    #[cfg(feature = "decimal")]
    Decimal(Decimal),
    Date(Date),
    Time(Time),
    DateTime(DateTime),
    Duration(Duration),
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
        Value::Int(Integer::new(v))
    }

    /// Create a float value
    #[inline]
    pub fn float(v: f64) -> Self {
        Value::Float(Float::new(v))
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
        matches!(self, Value::Int(_))
    }

    /// Check if value is float
    #[inline]
    pub const fn is_float(&self) -> bool {
        matches!(self, Value::Float(_))
    }

    /// Check if value is any numeric type
    #[inline]
    pub const fn is_numeric(&self) -> bool {
        match self {
            Value::Int(_) | Value::Float(_) => true,
            #[cfg(feature = "decimal")]
            Value::Decimal(_) => true,
            _ => false,
        }
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

    /// Get the type name as string
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Int(_) => "integer",
            Value::Float(_) => "float",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
            Value::Bytes(_) => "bytes",
            #[cfg(feature = "decimal")]
            Value::Decimal(_) => "decimal",
            Value::Date(_) => "date",
            Value::Time(_) => "time",
            Value::DateTime(_) => "datetime",
            Value::Duration(_) => "duration",
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
            Value::Int(i) => Some(i.value()),
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
            Value::Float(f) => Some(f.value()),
            Value::Int(i) => Some(i.value() as f64),
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
                let next = obj.get(*first)?;
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
    pub fn set_path(&mut self, path: &str, value: Value) -> Result<(), ValueError> {
        let parts: Vec<&str> = path.split('.').collect();
        self.set_path_segments(&parts, value)
    }

    /// Set value by path segments
    pub fn set_path_segments(&mut self, segments: &[&str], value: Value) -> Result<(), ValueError> {
        if segments.is_empty() {
            *self = value;
            return Ok(());
        }

        let (first, rest) = segments.split_first()
            .ok_or_else(|| ValueError::custom("Empty path segments"))?;

        if rest.is_empty() {
            // Last segment - set the value
            match self {
                Value::Object(obj) => {
                    let new_obj = obj.insert(first.to_string(), value);
                    *obj = new_obj;
                    Ok(())
                }
                Value::Array(arr) => {
                    let index = first.parse::<usize>()
                        .map_err(|_| ValueError::invalid_format("array index", *first))?;
                    if index >= arr.len() {
                        return Err(ValueError::index_out_of_bounds(index, arr.len()));
                    }
                    let new_arr = arr.set(index, value)
                        .map_err(|e| ValueError::custom(format!("array set error: {:?}", e)))?;
                    *arr = new_arr;
                    Ok(())
                }
                _ => Err(ValueError::unsupported_operation("set_path", self.type_name()))
            }
        } else {
            // Navigate deeper
            match self {
                Value::Object(obj) => {
                    let mut next_val = obj.get(*first).cloned().unwrap_or(Value::Object(Object::new()));
                    next_val.set_path_segments(rest, value)?;
                    let new_obj = obj.insert(first.to_string(), next_val);
                    *obj = new_obj;
                    Ok(())
                }
                Value::Array(arr) => {
                    let index = first.parse::<usize>()
                        .map_err(|_| ValueError::invalid_format("array index", *first))?;
                    if index >= arr.len() {
                        return Err(ValueError::index_out_of_bounds(index, arr.len()));
                    }
                    let mut elem = arr.get(index).cloned().ok_or_else(|| ValueError::index_out_of_bounds(index, arr.len()))?;
                    elem.set_path_segments(rest, value)?;
                    let new_arr = arr.set(index, elem)
                        .map_err(|e| ValueError::custom(format!("array set error: {:?}", e)))?;
                    *arr = new_arr;
                    Ok(())
                }
                _ => Err(ValueError::unsupported_operation("set_path", self.type_name()))
            }
        }
    }

    // ==================== Conversions ====================

    /// Try to coerce value to string
    pub fn coerce_to_string(&self) -> String {
        match self {
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::String(s) => s.to_string(),
            Value::Array(a) => format!("[array of {} items]", a.len()),
            Value::Object(o) => format!("[object with {} keys]", o.len()),
            Value::Bytes(b) => format!("[{} bytes]", b.len()),
            #[cfg(feature = "decimal")]
            Value::Decimal(d) => d.to_string(),
            Value::Date(d) => d.to_string(),
            Value::Time(t) => t.to_string(),
            Value::DateTime(dt) => dt.to_string(),
            Value::Duration(d) => d.to_string(),
        }
    }

    /// Try to coerce value to boolean
    pub fn coerce_to_bool(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => b.value(),
            Value::Int(i) => i.value() != 0,
            Value::Float(f) => f.value() != 0.0 && !f.is_nan(),
            Value::String(s) => !s.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => !o.is_empty(),
            Value::Bytes(b) => !b.is_empty(),
            #[cfg(feature = "decimal")]
            Value::Decimal(d) => !d.is_zero(),
            Value::Date(_) | Value::Time(_) | Value::DateTime(_) | Value::Duration(_) => true,
        }
    }

    /// Try to coerce value to number
    pub fn coerce_to_number(&self) -> Option<f64> {
        match self {
            Value::Bool(b) => Some(if b.value() { 1.0 } else { 0.0 }),
            Value::Int(i) => Some(i.value() as f64),
            Value::Float(f) => Some(f.value()),
            Value::String(s) => s.as_str().parse().ok(),
            #[cfg(feature = "decimal")]
            Value::Decimal(d) => Some(d.to_f64()),
            _ => None,
        }
    }

    // ==================== Collection Operations ====================

    /// Get the length/size of the value
    pub fn len(&self) -> usize {
        match self {
            Value::String(s) => s.len(),
            Value::Array(a) => a.len(),
            Value::Object(o) => o.len(),
            Value::Bytes(b) => b.len(),
            _ => 0,
        }
    }

    /// Check if value is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Merge two values (for objects, concatenate for arrays)
    pub fn merge(&mut self, other: Value) -> Result<(), ValueError> {
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
            (s, o) => Err(ValueError::incompatible_types(s.type_name(), o.type_name()))
        }
    }

    /// Deep clone the value
    pub fn deep_clone(&self) -> Self {
        self.clone()
    }
}

// ==================== Display Implementation ====================

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => write!(f, "null"),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Int(i) => write!(f, "{}", i),
            Value::Float(fl) => write!(f, "{}", fl),
            Value::String(s) => write!(f, "\"{}\"", s),
            Value::Array(a) => {
                write!(f, "[")?;
                for (i, item) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", item)?;
                }
                write!(f, "]")
            }
            Value::Object(o) => {
                write!(f, "{{")?;
                for (i, (k, v)) in o.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "\"{}\": {}", k, v)?;
                }
                write!(f, "}}")
            }
            Value::Bytes(b) => write!(f, "<{} bytes>", b.len()),
            #[cfg(feature = "decimal")]
            Value::Decimal(d) => write!(f, "{}", d),
            Value::Date(d) => write!(f, "{}", d),
            Value::Time(t) => write!(f, "{}", t),
            Value::DateTime(dt) => write!(f, "{}", dt),
            Value::Duration(d) => write!(f, "{}", d),
        }
    }
}

// ==================== Default Implementation ====================

impl Default for Value {
    fn default() -> Self {
        Value::Null
    }
}

// ==================== PartialEq Implementation ====================

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => a == b,
            (Value::Object(a), Value::Object(b)) => a == b,
            (Value::Bytes(a), Value::Bytes(b)) => a == b,
            #[cfg(feature = "decimal")]
            (Value::Decimal(a), Value::Decimal(b)) => a == b,
            (Value::Date(a), Value::Date(b)) => a == b,
            (Value::Time(a), Value::Time(b)) => a == b,
            (Value::DateTime(a), Value::DateTime(b)) => a == b,
            (Value::Duration(a), Value::Duration(b)) => a == b,
            _ => false,
        }
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

impl From<Option<Value>> for Value {
    fn from(v: Option<Value>) -> Self {
        v.unwrap_or(Value::Null)
    }
}