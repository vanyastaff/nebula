---

# nebula-derive

## Purpose

`nebula-derive` provides procedural macros to reduce boilerplate code when creating nodes and defining parameters. It enables compile-time validation and automatic code generation.

## Responsibilities

- Generate parameter collection code
- Generate action trait implementations  
- Validate attributes at compile time
- Generate serialization/deserialization code
- Create builder patterns automatically

## Architecture

### Macro Types

```rust
// Function-like macros
#[proc_macro]
pub fn node(input: TokenStream) -> TokenStream;

// Derive macros
#[proc_macro_derive(Parameters, attributes(param, validate, display))]
pub fn derive_parameters(input: TokenStream) -> TokenStream;

#[proc_macro_derive(Action, attributes(action, node))]
pub fn derive_action(input: TokenStream) -> TokenStream;

// Attribute macros
#[proc_macro_attribute]
pub fn trigger(args: TokenStream, input: TokenStream) -> TokenStream;
```

### Core Components

```rust
mod parameters;    // Parameter derive implementation
mod action;       // Action derive implementation  
mod validation;   // Compile-time validation
mod utils;        // Shared utilities
mod error;        // Error handling
```

## Derive Macros

### Parameters Derive

```rust
#[derive(Parameters)]
struct MyNodeParams {
    #[param(
        label = "API Key",
        description = "Your API key for authentication",
        required = true,
        secret = true
    )]
    api_key: String,
    
    #[param(
        label = "Timeout",
        description = "Request timeout in seconds",
        default = 30,
        min = 1,
        max = 300
    )]
    timeout: u32,
    
    #[param(
        label = "Retry Count", 
        default = 3,
        display(show_when(field = "advanced_mode", value = true))
    )]
    retry_count: u32,
}

// Generated code:
impl Parameters for MyNodeParams {
    fn parameter_collection() -> ParameterCollection {
        ParameterCollection::new()
            .add(TextParameter {
                key: Key::new("api_key"),
                metadata: ParameterMetadata {
                    label: "API Key",
                    description: Some("Your API key for authentication"),
                    required: true,
                },
                secret: true,
                ..Default::default()
            })
            .add(NumberParameter {
                key: Key::new("timeout"),
                metadata: ParameterMetadata {
                    label: "Timeout",
                    description: Some("Request timeout in seconds"),
                    required: false,
                },
                default: Some(30),
                min: Some(1),
                max: Some(300),
                ..Default::default()
            })
            // ...
    }
    
    fn from_values(values: HashMap<Key, ParameterValue>) -> Result<Self, Error> {
        Ok(Self {
            api_key: values.get(&Key::new("api_key"))
                .ok_or_else(|| Error::MissingRequired("api_key"))?
                .as_string()?,
            timeout: values.get(&Key::new("timeout"))
                .map(|v| v.as_number())
                .transpose()?
                .unwrap_or(30),
            // ...
        })
    }
}
```

### Action Derive

```rust
#[derive(Action)]
#[action(
    id = "http_request",
    name = "HTTP Request",
    category = "Network",
    version = "1.0.0"
)]
struct HttpRequestNode {
    #[parameters]
    params: HttpRequestParams,
}

// Generated code:
impl Action for HttpRequestNode {
    type Input = WorkflowDataItem;
    type Output = WorkflowDataItem;
    type Error = NodeError;
    
    fn metadata(&self) -> ActionMetadata {
        ActionMetadata {
            id: "http_request",
            name: "HTTP Request",
            category: "Network",
            version: Version::parse("1.0.0").unwrap(),
            ..Default::default()
        }
    }
    
    fn parameters(&self) -> &impl Parameters {
        &self.params
    }
}
```

## Display Control

### Conditional Display

```rust
#[derive(Parameters)]
struct ConditionalParams {
    #[param(label = "Mode")]
    mode: String,
    
    #[display(show_when(field = "mode", value = "advanced"))]
    advanced_options: AdvancedOptions,
    
    #[display(hide_when(field = "mode", value = "simple"))]
    expert_settings: ExpertSettings,
    
    #[display(show_when(
        condition = "any",
        rules = [
            (field = "mode", value = "custom"),
            (field = "enable_custom", value = true)
        ]
    ))]
    custom_config: String,
}
```

## Special Attributes

### Mode Parameters

```rust
#[derive(Parameters)]
struct ModeExample {
    #[mode(
        text(key = "manual", label = "Manual Input", placeholder = "Enter ID"),
        list(key = "select", label = "Select from List", options_from = "load_options"),
        expression(key = "dynamic", label = "Dynamic Expression")
    )]
    user_selection: String,
}

fn load_options() -> Vec<SelectOption> {
    vec![
        SelectOption::new("user1", "John Doe"),
        SelectOption::new("user2", "Jane Smith"),
    ]
}
```

### Array Parameters

```rust
#[derive(Parameters)]
struct ArrayExample {
    #[param(
        type = "array",
        label = "Tags",
        min_items = 1,
        max_items = 10,
        unique = true
    )]
    tags: Vec<String>,
    
    #[param(
        type = "array",
        label = "Headers",
        item_type = "object",
        item_schema = HeaderSchema
    )]
    headers: Vec<Header>,
}
```

## Error Handling

### Compile-time Errors

```rust
// This will fail at compile time
#[derive(Parameters)]
struct InvalidParams {
    #[validate(required)]  // Error: required on Option<T>
    optional_field: Option<String>,
    
    #[param(min = 10, max = 5)]  // Error: min > max
    invalid_range: u32,
}
```

### Error Messages

```rust
// Custom error messages
#[derive(Parameters)]
struct CustomErrors {
    #[validate(
        min_length = 8,
        message = "Password must be at least 8 characters long"
    )]
    password: String,
    
    #[validate(
        custom = "validate_complex",
        message = "Value must match the required format"
    )]
    complex_field: String,
}
```

## Performance Considerations

- All validation code is generated at compile time
- No runtime reflection or dynamic dispatch
- Minimal allocations in generated code
- Efficient parameter collection building

## Testing

### Unit Tests

```rust
#[test]
fn test_parameter_generation() {
    let collection = MyNodeParams::parameter_collection();
    assert_eq!(collection.len(), 3);
    assert!(collection.get("api_key").is_some());
}

#[test]
fn test_from_values() {
    let mut values = HashMap::new();
    values.insert(Key::new("api_key"), ParameterValue::String("test_key".into()));
    
    let params = MyNodeParams::from_values(values).unwrap();
    assert_eq!(params.api_key, "test_key");
    assert_eq!(params.timeout, 30); // default value
}
```

### Compile Tests

```rust
// tests/compile_fail/invalid_required.rs
#[derive(Parameters)]
struct Invalid {
    #[validate(required)]
    //~^ ERROR: 'required' validator cannot be used on Option<T> types
    field: Option<String>,
}
```

---

