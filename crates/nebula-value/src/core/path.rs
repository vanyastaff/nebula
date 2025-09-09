use crate::Value;
use crate::error::ValueError;
use crate::types::Object;
use std::fmt;

/// Represents a segment in a value path
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PathSegment {
    /// Object key
    Key(String),
    /// Array index
    Index(usize),
    /// Wildcard for iteration
    Wildcard,
    /// Recursive descent
    Recursive,
}

impl PathSegment {
    /// Parse a segment from string
    pub fn parse(s: &str) -> Result<Self, ValueError> {
        if s.is_empty() {
            return Err(ValueError::invalid_format("path segment", s));
        }

        match s {
            "*" => Ok(Self::Wildcard),
            "**" => Ok(Self::Recursive),
            _ => {
                // Try to parse as array index
                if let Ok(index) = s.parse::<usize>() {
                    Ok(Self::Index(index))
                } else {
                    Ok(Self::Key(s.to_string()))
                }
            }
        }
    }
}

impl fmt::Display for PathSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(k) => write!(f, "{}", k),
            Self::Index(i) => write!(f, "{}", i),
            Self::Wildcard => write!(f, "*"),
            Self::Recursive => write!(f, "**"),
        }
    }
}

/// Represents a path to navigate through Value structures
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValuePath {
    segments: Vec<PathSegment>,
}

impl ValuePath {
    /// Create an empty path
    pub fn new() -> Self {
        Self {
            segments: Vec::new(),
        }
    }

    /// Create a path from segments
    pub fn from_segments(segments: Vec<PathSegment>) -> Self {
        Self { segments }
    }

    /// Parse a path from string (e.g., "user.address.0.city")
    pub fn parse(path: &str) -> Result<Self, ValueError> {
        if path.is_empty() {
            return Ok(Self::new());
        }

        let segments = path
            .split('.')
            .map(PathSegment::parse)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self { segments })
    }

    /// Parse with bracket notation support (e.g., "users[0].name")
    pub fn parse_extended(path: &str) -> Result<Self, ValueError> {
        if path.is_empty() {
            return Ok(Self::new());
        }

        let mut segments = Vec::new();
        let mut current = String::new();
        let mut in_bracket = false;

        for ch in path.chars() {
            match ch {
                '[' => {
                    if !current.is_empty() {
                        segments.push(PathSegment::parse(&current)?);
                        current.clear();
                    }
                    in_bracket = true;
                }
                ']' => {
                    if in_bracket {
                        segments.push(PathSegment::parse(&current)?);
                        current.clear();
                        in_bracket = false;
                    } else {
                        return Err(ValueError::invalid_format("path", path));
                    }
                }
                '.' => {
                    if !in_bracket {
                        if !current.is_empty() {
                            segments.push(PathSegment::parse(&current)?);
                            current.clear();
                        }
                    } else {
                        current.push(ch);
                    }
                }
                _ => current.push(ch),
            }
        }

        if in_bracket {
            return Err(ValueError::invalid_format("path", path));
        }

        if !current.is_empty() {
            segments.push(PathSegment::parse(&current)?);
        }

        Ok(Self { segments })
    }

    /// Get the segments
    pub fn segments(&self) -> &[PathSegment] {
        &self.segments
    }

    /// Check if path is empty
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Get the length of the path
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Push a segment to the path
    pub fn push(&mut self, segment: PathSegment) {
        self.segments.push(segment);
    }

    /// Pop a segment from the path
    pub fn pop(&mut self) -> Option<PathSegment> {
        self.segments.pop()
    }

    /// Get a value at this path
    pub fn get<'a>(&self, value: &'a Value) -> Option<&'a Value> {
        self.get_from(value, 0)
    }

    fn get_from<'a>(&self, value: &'a Value, index: usize) -> Option<&'a Value> {
        if index >= self.segments.len() {
            return Some(value);
        }

        match &self.segments[index] {
            PathSegment::Key(key) => {
                if let Value::Object(obj) = value {
                    obj.get(key).and_then(|v| self.get_from(v, index + 1))
                } else {
                    None
                }
            }
            PathSegment::Index(idx) => {
                if let Value::Array(arr) = value {
                    arr.get(*idx).and_then(|v| self.get_from(v, index + 1))
                } else {
                    None
                }
            }
            PathSegment::Wildcard => {
                // Return None for wildcard in simple get
                // Use get_all for wildcard support
                None
            }
            PathSegment::Recursive => {
                // Return None for recursive in simple get
                // Use get_all for recursive support
                None
            }
        }
    }

    /// Get all values matching this path (supports wildcards)
    pub fn get_all<'a>(&self, value: &'a Value) -> Vec<&'a Value> {
        let mut results = Vec::new();
        self.get_all_from(value, 0, &mut results);
        results
    }

    fn get_all_from<'a>(&self, value: &'a Value, index: usize, results: &mut Vec<&'a Value>) {
        if index >= self.segments.len() {
            results.push(value);
            return;
        }

        match &self.segments[index] {
            PathSegment::Key(key) => {
                if let Value::Object(obj) = value
                    && let Some(v) = obj.get(key) {
                        self.get_all_from(v, index + 1, results);
                    }
            }
            PathSegment::Index(idx) => {
                if let Value::Array(arr) = value
                    && let Some(v) = arr.get(*idx) {
                        self.get_all_from(v, index + 1, results);
                    }
            }
            PathSegment::Wildcard => match value {
                Value::Array(arr) => {
                    for item in arr.iter() {
                        self.get_all_from(item, index + 1, results);
                    }
                }
                Value::Object(obj) => {
                    for (_, val) in obj.iter() {
                        self.get_all_from(val, index + 1, results);
                    }
                }
                _ => {}
            },
            PathSegment::Recursive => {
                // Add current value
                self.get_all_from(value, index + 1, results);

                // Recursively search all children
                match value {
                    Value::Array(arr) => {
                        for item in arr.iter() {
                            self.get_all_from(item, index, results);
                        }
                    }
                    Value::Object(obj) => {
                        for (_, val) in obj.iter() {
                            self.get_all_from(val, index, results);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Set a value at this path
    pub fn set(&self, value: &mut Value, new_value: Value) -> Result<(), ValueError> {
        self.set_from(value, new_value, 0)
    }

    fn set_from(
        &self,
        value: &mut Value,
        new_value: Value,
        index: usize,
    ) -> Result<(), ValueError> {
        if index >= self.segments.len() {
            *value = new_value;
            return Ok(());
        }

        if index == self.segments.len() - 1 {
            // Last segment - set the value
            match &self.segments[index] {
                PathSegment::Key(key) => {
                    if let Value::Object(obj) = value {
                        let _ = obj.insert(key.clone(), new_value);
                        Ok(())
                    } else {
                        Err(ValueError::type_mismatch("object", value.type_name()))
                    }
                }
                PathSegment::Index(idx) => {
                    if let Value::Array(arr) = value {
                        if *idx >= arr.len() {
                            return Err(ValueError::index_out_of_bounds(*idx, arr.len()));
                        }
                        let new_arr = arr
                            .set(*idx, new_value)
                            .map_err(|e| ValueError::custom(format!("array set error: {:?}", e)))?;
                        *arr = new_arr;
                        Ok(())
                    } else {
                        Err(ValueError::type_mismatch("array", value.type_name()))
                    }
                }
                _ => Err(ValueError::unsupported_operation(
                    "set with wildcard",
                    "path",
                )),
            }
        } else {
            // Navigate deeper
            match &self.segments[index] {
                PathSegment::Key(key) => {
                    if let Value::Object(obj) = value {
                        let mut next_val = obj
                            .get(key)
                            .cloned()
                            .unwrap_or(Value::Object(Object::new()));
                        self.set_from(&mut next_val, new_value, index + 1)?;
                        let new_obj = obj.insert(key.clone(), next_val);
                        *obj = new_obj;
                        Ok(())
                    } else {
                        Err(ValueError::type_mismatch("object", value.type_name()))
                    }
                }
                PathSegment::Index(idx) => {
                    if let Value::Array(arr) = value {
                        if *idx >= arr.len() {
                            return Err(ValueError::index_out_of_bounds(*idx, arr.len()));
                        }
                        let mut elem = arr
                            .get(*idx)
                            .cloned()
                            .ok_or_else(|| ValueError::index_out_of_bounds(*idx, arr.len()))?;
                        self.set_from(&mut elem, new_value, index + 1)?;
                        let new_arr = arr
                            .set(*idx, elem)
                            .map_err(|e| ValueError::custom(format!("array set error: {:?}", e)))?;
                        *arr = new_arr;
                        Ok(())
                    } else {
                        Err(ValueError::type_mismatch("array", value.type_name()))
                    }
                }
                _ => Err(ValueError::unsupported_operation(
                    "set with wildcard",
                    "path",
                )),
            }
        }
    }

    /// Delete value at this path
    pub fn delete(&self, value: &mut Value) -> Result<bool, ValueError> {
        if self.segments.is_empty() {
            return Ok(false);
        }

        let (last, path) = self.segments.split_last().unwrap();

        // Navigate to parent
        let parent = if path.is_empty() {
            value
        } else {
            let parent_path = Self::from_segments(path.to_vec());
            parent_path
                .get_mut(value)
                .ok_or_else(|| ValueError::custom("Parent path not found"))?
        };

        // Delete from parent
        match last {
            PathSegment::Key(key) => {
                if let Value::Object(obj) = parent {
                    match obj.remove(key) {
                        Ok((new_obj, _)) => {
                            *obj = new_obj;
                            Ok(true)
                        }
                        Err(_) => Ok(false),
                    }
                } else {
                    Err(ValueError::type_mismatch("object", parent.type_name()))
                }
            }
            PathSegment::Index(idx) => {
                if let Value::Array(arr) = parent {
                    if *idx < arr.len() {
                        if let Ok((new_arr, _)) = arr.remove(*idx) {
                            *arr = new_arr;
                            Ok(true)
                        } else {
                            Ok(false)
                        }
                    } else {
                        Ok(false)
                    }
                } else {
                    Err(ValueError::type_mismatch("array", parent.type_name()))
                }
            }
            _ => Err(ValueError::unsupported_operation(
                "delete with wildcard",
                "path",
            )),
        }
    }

    /// Get mutable reference to value at path (not supported for persistent structures)
    pub fn get_mut<'a>(&self, _value: &'a mut Value) -> Option<&'a mut Value> {
        None
    }

    fn get_mut_from<'a>(&self, _value: &'a mut Value, _index: usize) -> Option<&'a mut Value> {
        None
    }
}

impl fmt::Display for ValuePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, segment) in self.segments.iter().enumerate() {
            if i > 0 {
                write!(f, ".")?;
            }
            write!(f, "{}", segment)?;
        }
        Ok(())
    }
}

impl Default for ValuePath {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_parse() {
        let path = ValuePath::parse("user.address.city").unwrap();
        assert_eq!(path.segments.len(), 3);

        let path = ValuePath::parse("items.0.name").unwrap();
        assert_eq!(path.segments[1], PathSegment::Index(0));

        let path = ValuePath::parse("data.*.id").unwrap();
        assert_eq!(path.segments[1], PathSegment::Wildcard);
    }

    #[test]
    fn test_path_parse_extended() {
        let path = ValuePath::parse_extended("users[0].name").unwrap();
        assert_eq!(path.segments.len(), 3);
        assert_eq!(path.segments[1], PathSegment::Index(0));

        let path = ValuePath::parse_extended("data[field.with.dots]").unwrap();
        assert_eq!(path.segments.len(), 2);
    }
}
