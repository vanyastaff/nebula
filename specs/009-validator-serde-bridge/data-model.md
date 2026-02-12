# Data Model: Validator Serde Bridge

**Feature**: 009-validator-serde-bridge
**Date**: 2026-02-11

## Entities

### PathSegment (enum)

Represents a single step in a JSON path traversal.

| Variant | Fields | Description |
|---------|--------|-------------|
| `Key` | `String` | Object key access (e.g., `"server"`) |
| `Index` | `usize` | Array index access (e.g., `0`) |

### JsonPath (struct)

Parsed representation of a field path string like `"server.hosts[0].port"`.

| Field | Type | Description |
|-------|------|-------------|
| `segments` | `Vec<PathSegment>` | Ordered list of path segments |
| `original` | `String` | Original path string for error messages |

**Relationships:**
- Contains 0..N `PathSegment` values
- Used by `JsonField` to traverse `serde_json::Value`

**Validation Rules:**
- Path must not be empty
- Array indices must be valid non-negative integers
- Bracket notation must be properly closed

**Operations:**
- `parse(path: &str) -> Result<JsonPath, ValidationError>` — parses `"a.b[0].c"` into segments
- `resolve<'a>(&self, root: &'a Value) -> Result<&'a Value, ValidationError>` — traverses JSON tree
- `Display` — formats back to human-readable path for error messages

### JsonField (struct / combinator)

A combinator that extracts a value from a JSON object by path and validates it with an inner validator.

| Field | Type | Description |
|-------|------|-------------|
| `path` | `JsonPath` | Path to the target field |
| `validator` | `V` | Inner validator to apply to the extracted value |
| `required` | `bool` | Whether the field must exist (default: true) |

**Relationships:**
- Contains one `JsonPath`
- Contains one inner `Validate` implementor
- Implements `Validate<Input = serde_json::Value>`

**State Transitions:** N/A (stateless)

### AsValidatable Implementations (trait impls, not stored types)

These are trait implementations, not data types, but they define the type mapping:

| Source (`Value` variant) | Target type | Conversion |
|--------------------------|-------------|------------|
| `Value::String(s)` | `str` | `s.as_str()` |
| `Value::Number(n)` | `i64` | `n.as_i64()` or type mismatch |
| `Value::Number(n)` | `f64` | `n.as_f64()` or type mismatch |
| `Value::Bool(b)` | `bool` | `*b` |
| `Value::Array(arr)` | `[Value]` | `arr.as_slice()` |
| `Value::Null` | any | Always returns type mismatch error |
| `Value::Object(_)` | any scalar | Always returns type mismatch error |

## Error Codes

New error codes introduced by the bridge:

| Code | Message Pattern | When |
|------|-----------------|------|
| `type_mismatch` | `"Expected {expected}, got {actual}"` | Value variant doesn't match validator input type |
| `path_not_found` | `"Path '{path}' not found"` | JSON path traversal reaches missing key/index |
| `invalid_path` | `"Invalid path syntax: '{path}'"` | Path string fails to parse |
| `index_out_of_bounds` | `"Index {index} out of bounds at '{path}'"` | Array index exceeds array length |
