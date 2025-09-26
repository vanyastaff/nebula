# nebula-parameter

A comprehensive, type-safe parameter system for the Nebula workflow engine, providing flexible parameter definition, validation, and conditional display capabilities.

## Table of Contents

1. [Overview](#overview)
2. [Core Concepts](#core-concepts)
3. [Architecture](#architecture)
4. [Parameter Types](#parameter-types)
5. [Validation System](#validation-system)
6. [Display System](#display-system)
7. [Parameter Collections](#parameter-collections)
8. [Advanced Features](#advanced-features)
9. [Best Practices](#best-practices)
10. [API Reference](#api-reference)

## Overview

The nebula-parameter system provides a robust framework for defining, validating, and managing parameters in workflow nodes. It emphasizes type safety, extensibility, and clear separation of concerns between data storage, validation logic, and UI presentation.

### Key Features

- **Type Safety**: Strongly typed parameters with compile-time guarantees
- **Comprehensive Validation**: Flexible validation system with built-in and custom rules
- **Conditional Display**: Show/hide parameters based on dynamic conditions
- **Security First**: Secure parameter types with automatic memory zeroing
- **Builder Pattern**: Ergonomic API for parameter construction
- **Rich UI Options**: Separate UI configuration from data logic
- **Parameter Collections**: Support for groups, lists, and conditional parameters
- **Full Serialization**: Complete serde support with smart defaults

### Design Philosophy

The parameter system follows these core principles:

1. **Separation of Concerns**: Data, validation, display, and UI are separate
2. **Composability**: Complex behaviors built from simple, composable parts
3. **Zero-Cost Abstractions**: No runtime overhead for type safety
4. **Progressive Disclosure**: Simple things are simple, complex things are possible
5. **Fail Fast**: Validation errors caught as early as possible

## Core Concepts

### Parameter Anatomy

Every parameter consists of several components:

```rust
pub struct TextParameter {
    // Core identity and metadata
    pub metadata: ParameterMetadata,

    // The actual value (optional)
    pub value: Option<String>,

    // Default value (optional)
    pub default: Option<String>,

    // Validation rules (optional)
    pub validation: Option<ParameterValidation>,

    // Display conditions (optional)
    pub display: Option<ParameterDisplay>,

    // UI configuration
    pub ui_options: TextUiOptions,
}
```

### Metadata

Parameter metadata provides core information about the parameter:

```rust
pub struct ParameterMetadata {
    /// Unique identifier within the node
    pub key: Cow<'static, str>,

    /// Human-readable name
    pub name: Cow<'static, str>,

    /// Whether this parameter is required
    pub required: bool,

    /// Detailed description
    pub description: Option<Cow<'static, str>>,

    /// Placeholder text for empty inputs
    pub placeholder: Option<Cow<'static, str>>,

    /// Help text or usage hints
    pub hint: Option<Cow<'static, str>>,
}
```

### Parameter Lifecycle

1. **Definition**: Parameter is defined with metadata and options
2. **Validation**: Rules are applied when values are set
3. **Display Evaluation**: Visibility determined by conditions
4. **Value Management**: Get/set/clear operations with type safety
5. **Serialization**: Convert to/from JSON for storage/transmission

## Architecture

### Trait Hierarchy

The parameter system is built on a hierarchy of traits:

```rust
/// Base trait for all parameters
pub trait ParameterType {
    fn kind(&self) -> ParameterKind;
    fn metadata(&self) -> &ParameterMetadata;
}

/// Parameters that can store values
pub trait HasValue: ParameterType {
    type Value: Clone + PartialEq + Debug + 'static;

    fn get_value(&self) -> Option<&Self::Value>;
    fn set_value_unchecked(&mut self, value: Self::Value) -> Result<(), ParameterError>;
    // ... many more methods
}

/// Parameters that support validation
pub trait Validatable: HasValue {
    fn validate(&self, value: &Self::Value) -> Result<(), ParameterError>;
}

/// Parameters that support conditional display
pub trait Displayable: ParameterType {
    fn display(&self) -> Option<&ParameterDisplay>;
    fn should_display(&self, context: &DisplayContext) -> bool;
}
```

### Type System

Parameters use Rust's type system for safety:

- Strong typing prevents type confusion
- Option<T> for optional values
- Result<T, E> for fallible operations
- Phantom types for compile-time guarantees
- Zero-cost newtype wrappers

## Parameter Types

### TextParameter

For single or multi-line text input:

```rust
// Simple text input
let username = TextParameter::new(
    ParameterMetadata::new("username", "Username")
);

// Multi-line with validation
let description = TextParameter::builder()
    .metadata(
        ParameterMetadata::builder()
            .key("description")
            .name("Description")
            .placeholder("Enter description...")
            .build()?
    )
    .ui_options(TextUiOptions::multi_line(5))
    .validation(
        ParameterValidation::builder()
            .min_length(10)
            .max_length(500)
            .build()
    )
    .build()?;

// With input mask
let phone = TextParameter::builder()
    .metadata(metadata)
    .ui_options(
        TextUiOptions::builder()
            .mask("(999) 999-9999")
            .build()
    )
    .build()?;
```

### SecretParameter

Secure parameter with automatic memory zeroing:

```rust
// Password with strength requirements
let password = SecretParameter::password(
    ParameterMetadata::new("password", "Password")
).with_validation(
    ParameterValidation::builder()
        .min_length(8)
        .match_regex(r"^(?=.*[a-z])(?=.*[A-Z])(?=.*\d)(?=.*[@$!%*?&])")
        .build()
);

// API key
let api_key = SecretParameter::api_key(
    ParameterMetadata::new("api_key", "API Key")
);

// Custom secret
let secret = SecretParameter::builder()
    .metadata(metadata)
    .ui_options(
        SecretUiOptions::builder()
            .show_toggle(true)
            .show_strength_meter(true)
            .build()
    )
    .build()?;
```

Features:
- Automatic memory zeroing on drop (via `zeroize`)
- Masked display in logs and serialization
- Secure string wrapper prevents accidental exposure
- Built-in password strength validation

### NumberParameter

Numeric input with formatting options:

```rust
// Simple number
let age = NumberParameter::builder()
    .metadata(ParameterMetadata::new("age", "Age"))
    .ui_options(
        NumberUiOptions::builder()
            .min(0.0)
            .max(150.0)
            .format(NumberFormat::Integer)
            .build()
    )
    .build()?;

// Currency input
let price = NumberParameter::builder()
    .metadata(ParameterMetadata::new("price", "Price"))
    .ui_options(
        NumberUiOptions::builder()
            .format(NumberFormat::Currency { code: "USD".into() })
            .min(0.0)
            .step(0.01)
            .build()
    )
    .build()?;

// Percentage with slider
let confidence = NumberParameter::builder()
    .metadata(ParameterMetadata::new("confidence", "Confidence"))
    .ui_options(
        NumberUiOptions::builder()
            .format(NumberFormat::Percentage)
            .min(0.0)
            .max(100.0)
            .slider(true)
            .build()
    )
    .build()?;
```

Number formats:
- `Decimal { precision }` - Fixed decimal places
- `Integer` - Whole numbers only
- `Percentage` - Shows as percentage
- `Currency { code }` - Currency formatting
- `Scientific { precision }` - Scientific notation

### BooleanParameter

Boolean input with various display options:

```rust
// Simple checkbox
let enabled = BooleanParameter::new(
    ParameterMetadata::new("enabled", "Enable Feature")
);

// Switch with custom labels
let production = BooleanParameter::builder()
    .metadata(metadata)
    .ui_options(
        BooleanUiOptions::builder()
            .display_type(BooleanDisplayType::Switch)
            .true_label("Production")
            .false_label("Development")
            .build()
    )
    .build()?;
```

### SelectParameter

Single selection from a list of options:

```rust
// Static options
let country = SelectParameter::builder()
    .metadata(ParameterMetadata::new("country", "Country"))
    .options(vec![
        SelectOption::new("us", "United States"),
        SelectOption::new("uk", "United Kingdom"),
        SelectOption::new("ca", "Canada"),
    ])
    .build()?;

// With option groups
let timezone = SelectParameter::builder()
    .metadata(metadata)
    .options(vec![
        SelectOption::builder()
            .value("EST")
            .label("Eastern Time")
            .group("Americas")
            .build(),
        SelectOption::builder()
            .value("PST")
            .label("Pacific Time")
            .group("Americas")
            .build(),
        SelectOption::builder()
            .value("GMT")
            .label("Greenwich Mean Time")
            .group("Europe")
            .build(),
    ])
    .build()?;

// Dynamic options from endpoint
let users = SelectParameter::builder()
    .metadata(metadata)
    .dynamic_options(
        OptionSource::Endpoint {
            url: "https://api.example.com/users".into(),
            headers: HashMap::new(),
        }
    )
    .build()?;
```

### MultiSelectParameter

Multiple selection with constraints:

```rust
let tags = MultiSelectParameter::builder()
    .metadata(ParameterMetadata::new("tags", "Tags"))
    .options(tag_options)
    .min_items(1)
    .max_items(5)
    .validation(
        ParameterValidation::builder()
            .with_rule(ParameterCondition::custom(|value| {
                // Custom validation logic
                Ok(())
            }))
            .build()
    )
    .build()?;
```

### DateTimeParameter

Date and time input with timezone support:

```rust
// Date only
let birthday = DateTimeParameter::builder()
    .metadata(ParameterMetadata::new("birthday", "Birthday"))
    .ui_options(
        DateTimeUiOptions::builder()
            .mode(DateTimeMode::DateOnly)
            .max(DateTime::now())
            .build()
    )
    .build()?;

// With presets
let schedule = DateTimeParameter::builder()
    .metadata(metadata)
    .ui_options(
        DateTimeUiOptions::builder()
            .mode(DateTimeMode::DateTime)
            .timezone(TimezoneHandling::UserLocal)
            .presets(vec![
                DateTimePreset::now(),
                DateTimePreset::start_of_day(),
                DateTimePreset::in_hours(1),
                DateTimePreset::tomorrow(),
            ])
            .build()
    )
    .build()?;
```

### FileParameter

File upload with validation:

```rust
let avatar = FileParameter::builder()
    .metadata(ParameterMetadata::new("avatar", "Profile Picture"))
    .ui_options(
        FileUiOptions::builder()
            .accept(vec!["image/jpeg", "image/png"])
            .max_size(5 * 1024 * 1024) // 5MB
            .preview(true)
            .build()
    )
    .validation(
        ParameterValidation::builder()
            .with_rule(ParameterCondition::custom(|file: &FileReference| {
                // Validate image dimensions
                Ok(())
            }))
            .build()
    )
    .build()?;
```

## Validation System

### Validation Architecture

The validation system is built on conditions that can be composed:

```rust
pub enum ParameterCondition {
    // Comparison
    Eq(Value),
    NotEq(Value),
    Gt(Value),
    Gte(Value),
    Lt(Value),
    Lte(Value),
    Between { from: Value, to: Value },

    // String operations
    StartsWith(Value),
    EndsWith(Value),
    Contains(Value),
    Regex(Value),
    StringMinLength(usize),
    StringMaxLength(usize),

    // Existence
    IsEmpty,
    IsNotEmpty,

    // Logical
    And(Vec<ParameterCondition>),
    Or(Vec<ParameterCondition>),
    Not(Box<ParameterCondition>),
}
```

### Building Validations

Use the builder pattern for readable validation rules:

```rust
let validation = ParameterValidation::builder()
    // Value constraints
    .greater_than(0)
    .less_than_or_equal(100)

    // String constraints
    .not_empty()
    .min_length(3)
    .max_length(50)
    .match_regex(r"^[a-zA-Z0-9_]+$")

    // Complex conditions
    .all(vec![
        ParameterCondition::StringMinLength(8),
        ParameterCondition::Or(vec![
            ParameterCondition::Contains(json!("@")),
            ParameterCondition::Contains(json!("+")),
        ]),
    ])
    .build();
```

### Built-in Validators

Common validation patterns are provided:

```rust
// Email validation
let email_validation = validators::email();

// URL validation
let url_validation = validators::url();

// String with constraints
let username_validation = validators::string(
    Some(3),  // min_length
    Some(20), // max_length
    Some(r"^[a-zA-Z0-9_]+$") // pattern
);

// Number range
let percentage_validation = validators::number(
    Some(0.0),   // min
    Some(100.0), // max
    false        // integer_only
);
```

### Custom Validation

Implement custom validation logic:

```rust
// Inline custom validation
let validation = ParameterValidation::builder()
    .with_rule(ParameterCondition::custom(|value: &String| {
        if value.split('@').count() != 2 {
            return Err(ValidationError::Custom(
                "Email must contain exactly one @ symbol".into()
            ));
        }
        Ok(())
    }))
    .build();

// Reusable validator
pub fn validate_strong_password(value: &str) -> Result<(), ValidationError> {
    if value.len() < 8 {
        return Err(ValidationError::Custom("Password too short".into()));
    }

    let has_upper = value.chars().any(|c| c.is_uppercase());
    let has_lower = value.chars().any(|c| c.is_lowercase());
    let has_digit = value.chars().any(|c| c.is_digit(10));
    let has_special = value.chars().any(|c| "!@#$%^&*".contains(c));

    if !(has_upper && has_lower && has_digit && has_special) {
        return Err(ValidationError::Custom(
            "Password must contain uppercase, lowercase, digit, and special character".into()
        ));
    }

    Ok(())
}
```

### Validation Errors

Rich error information for better UX:

```rust
match parameter.validate(&value) {
    Ok(()) => println!("Valid!"),
    Err(ParameterError::ValidationErrors(errors)) => {
        for error in errors {
            match error {
                ParameterCheckError::StringLengthTooShort { min, actual } => {
                    println!("Too short: need {} chars, got {}", min, actual);
                }
                ParameterCheckError::RegexComparisonFailed { expected, actual } => {
                    println!("Pattern mismatch: expected '{}', got '{}'", expected, actual);
                }
                ParameterCheckError::NumericComparisonFailed { operator, expected, actual } => {
                    println!("Number {} check failed: expected {}, got {}", operator, expected, actual);
                }
                _ => println!("Validation failed: {}", error),
            }
        }
    }
    Err(e) => println!("Unexpected error: {}", e),
}
```

## Display System

### Display Traits

The display system uses traits for flexibility:

```rust
/// Base trait for conditional display
pub trait Displayable: ParameterType {
    fn display(&self) -> Option<&ParameterDisplay>;
    fn should_display(&self, context: &DisplayContext) -> bool;
    fn validate_display(&self, context: &DisplayContext) -> Result<(), ParameterDisplayError>;
    fn has_display_conditions(&self) -> bool;
    fn display_dependencies(&self) -> Vec<Key>;
}

/// Extended display operations
pub trait DisplayableExt: Displayable {
    fn set_display(&mut self, display: Option<ParameterDisplay>);
    fn add_display_condition(&mut self, property: Key, condition: ParameterCondition);
    fn clear_display_conditions(&mut self);
}

/// Reactive display behavior
pub trait DisplayReactive: Displayable {
    fn on_show(&mut self, context: &DisplayContext);
    fn on_hide(&mut self, context: &DisplayContext);
    fn on_display_change(&mut self, old_visible: bool, new_visible: bool, context: &DisplayContext);
}
```

### Display Context

Rich context for display evaluation:

```rust
pub struct DisplayContext {
    /// Current parameter values
    pub properties: HashMap<Key, Value>,

    /// User role/permissions
    pub user_role: Option<String>,

    /// Current UI mode
    pub ui_mode: Option<UiMode>,

    /// Additional metadata
    pub metadata: HashMap<String, Value>,
}

// Create context with builder pattern
let context = DisplayContext::new(current_values)
    .with_role("admin".to_string())
    .with_mode(UiMode::Advanced);
```

### Display Conditions

Build complex display logic:

```rust
// Simple conditions
let display = ParameterDisplay::builder()
    .show_when_equals("mode", "advanced")
    .hide_when_equals("debug", false)
    .build();

// Complex conditions
let display = ParameterDisplay::builder()
    .show_when("level", ParameterCondition::Gte(json!(5)))
    .show_when("role", ParameterCondition::Or(vec![
        ParameterCondition::Eq(json!("admin")),
        ParameterCondition::Eq(json!("developer")),
    ]))
    .hide_when("environment", ParameterCondition::Eq(json!("production")))
    .build();

// Using display chain for readability
let display = DisplayChain::show()
    .when("feature_flag", ParameterCondition::Eq(json!(true)))
    .when("user_level", ParameterCondition::Gt(json!(10)))
    .when_any_of("department", vec![
        ParameterCondition::Eq(json!("engineering")),
        ParameterCondition::Eq(json!("qa")),
    ])
    .build();
```

### Reactive Parameters

Parameters that respond to visibility changes:

```rust
impl DisplayReactive for SelectParameter {
    fn on_show(&mut self, context: &DisplayContext) {
        // Load options when becoming visible
        if self.options.is_dynamic() {
            self.refresh_options(context);
        }
    }

    fn on_hide(&mut self, _context: &DisplayContext) {
        // Clear cache to save memory
        self.clear_option_cache();
    }
}
```

## Legacy Features from old_parameter

> **Important Discovery**: The `old_parameter/` folder contains a comprehensive implementation with advanced features that are currently missing from the new implementation. These represent proven solutions that could be integrated.

### Missing Core Infrastructure

#### ParameterCollection - Comprehensive Parameter Management
The old implementation provided a robust `ParameterCollection` system:

**Key Features:**
- HashMap-based parameter storage with type-safe access
- Snapshot/restore functionality for state management
- Bulk operations and iteration support
- Error handling with detailed error types

**Core API:**
```rust
// From old_parameter/collection.rs
impl ParameterCollection {
    fn add(&mut self, parameter: ParameterType) -> Result<(), ParameterCollectionError>;
    fn get_as<P: Parameter>(&self, key: &Key) -> Result<&P, ParameterCollectionError>;
    fn snapshot(&self) -> HashMap<Key, Option<ParameterValue>>;
    fn load_snapshot(&mut self, snapshot: &HashMap<Key, Option<ParameterValue>>) -> Result<(), ParameterCollectionError>;
}
```

#### ParameterStore - Thread-Safe State Management
Advanced parameter storage with modification tracking:

**Key Features:**
- Thread-safe access using `Arc<RwLock<ParameterCollection>>`
- Override system for temporary parameter changes
- Modification tracking with `HashSet<Key>`
- Automatic serialization/deserialization

**Core API:**
```rust
// From old_parameter/store.rs
impl ParameterStore {
    fn get<T>(&self, key: &Key) -> Result<T, ParameterError>;
    fn set_override<T>(&mut self, key: &Key, value: T) -> Result<(), ParameterError>;
    fn get_modified_parameters(&self) -> &HashSet<Key>;
    fn has_modifications(&self) -> bool;
}
```

### Missing Parameter Types

#### Expirable Parameters - Time-Based Expiration
Sophisticated TTL (time-to-live) functionality:

**Key Features:**
- Automatic expiration checking using chrono
- TTL refresh capabilities
- Wrapper pattern around existing parameters
- Builder pattern support

**Core API:**
```rust
// From old_parameter/types/expirable.rs
impl ExpirableParameter {
    fn with_ttl(parameter: ParameterType, ttl: u64) -> Self;
    fn is_expired(&self) -> bool;
    fn refresh_ttl(&mut self) -> Result<(), ParameterError>;
    fn get_actual_value(&self) -> Option<Value>; // Returns None if expired
}
```

#### Group Parameters - Hierarchical Structure
Complex nested parameter structures:

**Key Features:**
- Contains child `ParameterCollection`
- Value distribution to child parameters
- Value collection from child parameters
- Support for multiple instances with constraints

**Core API:**
```rust
// From old_parameter/types/group.rs
impl GroupParameter {
    fn collect_values(&self) -> Result<Option<ParameterValue>, ParameterError>;
    fn set_value(&mut self, value: ParameterValue) -> Result<(), ParameterError>; // Distributes to children
}
```

#### UI Parameter Types
The old implementation included many UI-focused parameter types missing from current:

**Available Types:**
- `button` - Action triggers
- `checkbox` - Boolean with checkbox UI
- `radio` - Single selection from options
- `multi_select` - Multiple selection
- `textarea` - Multi-line text input
- `datetime`, `time` - Enhanced date/time handling
- `hidden` - Hidden parameters
- `notice` - Display-only information
- `routing` - Navigation parameters

**Enhanced Types:**
Current types exist but old versions had more features:
- `expirable` (12KB) - Complex expiration logic
- `group` (7KB) - Hierarchical parameter grouping
- `mode` (4KB) - Parameter mode switching

### Integration Opportunities

#### 1. Parameter Collection System
The current implementation lacks the comprehensive collection management found in `old_parameter/collection.rs`. This could provide:
- Better parameter organization
- State management capabilities
- Bulk operations support

#### 2. Thread-Safe Parameter Store
The `ParameterStore` from `old_parameter/store.rs` offers production-ready features:
- Concurrent access patterns
- Change tracking for UI updates
- Override system for temporary changes

#### 3. Time-Based Parameters
The `ExpirableParameter` system enables:
- Credential expiration handling
- Cache invalidation
- Session management

#### 4. Hierarchical Parameters
The `GroupParameter` system supports:
- Complex form structures
- Nested configuration sections
- Dynamic parameter sets

## Parameter Collections

### Parameter Groups

Organize related parameters:

```rust
let database_config = ParameterGroup::builder()
    .metadata(
        GroupMetadata::builder()
            .key("database")
            .name("Database Configuration")
            .description("Configure database connection")
            .icon("database")
            .build()
    )
    .parameters(vec![
        Parameter::Text(host),
        Parameter::Number(port),
        Parameter::Text(username),
        Parameter::Secret(password),
        Parameter::Boolean(use_ssl),
    ])
    .layout(GroupLayout::Vertical)
    .collapsible(true)
    .default_expanded(false)
    .display(
        ParameterDisplay::builder()
            .show_when_equals("use_database", true)
            .build()
    )
    .build();
```

### Parameter Lists

Dynamic collections of parameters:

```rust
let http_headers = ParameterList::builder()
    .metadata(
        ListMetadata::builder()
            .key("headers")
            .name("HTTP Headers")
            .description("Custom HTTP headers")
            .add_button_text("Add Header")
            .empty_text("No headers configured")
            .build()
    )
    .item_template(
        Parameter::Object(
            ObjectParameter::builder()
                .add_field("name", TextParameter::new(/* ... */))
                .add_field("value", TextParameter::new(/* ... */))
                .build()
        )
    )
    .min_items(0)
    .max_items(20)
    .default_items(vec![
        // Common default headers
    ])
    .build();
```

### Conditional Parameters

Parameters that change based on conditions:

```rust
let auth_config = ConditionalParameter::builder()
    .condition(ParameterCondition::Eq(json!("oauth2")))
    .then_parameters(vec![
        Parameter::Text(client_id),
        Parameter::Secret(client_secret),
        Parameter::Text(auth_url),
        Parameter::Text(token_url),
    ])
    .else_parameters(vec![
        Parameter::Text(username),
        Parameter::Secret(password),
    ])
    .build();
```

### Nested Collections

Complex parameter structures:

```rust
let workflow_config = ParameterCollection::Group(
    ParameterGroup::builder()
        .metadata(/* ... */)
        .parameters(vec![
            // Basic settings
            ParameterCollection::Group(basic_settings),

            // Advanced settings (conditional)
            ParameterCollection::Conditional(
                ConditionalParameter::builder()
                    .condition(ParameterCondition::Eq(json!("advanced")))
                    .then_parameters(vec![
                        ParameterCollection::Group(performance_settings),
                        ParameterCollection::Group(security_settings),
                        ParameterCollection::List(custom_rules),
                    ])
                    .build()
            ),
        ])
        .build()
);
```

## Advanced Features

### Cross-Parameter Validation

Validate relationships between parameters:

```rust
pub struct CrossParameterValidation {
    pub rules: Vec<CrossParameterRule>,
}

impl CrossParameterValidation {
    pub fn validate(&self, values: &HashMap<Key, ParameterValue>) -> Result<(), ValidationError> {
        for rule in &self.rules {
            rule.validate(values)?;
        }
        Ok(())
    }
}

// Example: Password confirmation
let password_confirmation = CrossParameterRule::builder()
    .parameters(vec!["password", "confirm_password"])
    .condition(CrossParameterCondition::Custom(Box::new(|values| {
        match (values.get("password"), values.get("confirm_password")) {
            (Some(p1), Some(p2)) => p1 == p2,
            _ => false,
        }
    })))
    .error_message("Passwords do not match")
    .build();

// Example: Mutually exclusive options
let exclusive_rule = CrossParameterRule::builder()
    .parameters(vec!["use_default", "custom_value"])
    .condition(CrossParameterCondition::MutuallyExclusive)
    .error_message("Cannot use both default and custom value")
    .build();
```

### Parameter Templates

Reusable parameter configurations:

```rust
pub struct ParameterTemplate {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub parameters: Vec<Parameter>,
    pub variables: HashMap<String, Value>,
}

// Define a template
let email_template = ParameterTemplate::builder()
    .id("email_input")
    .name("Email Input Template")
    .parameters(vec![
        Parameter::Text(
            TextParameter::builder()
                .metadata(
                    ParameterMetadata::builder()
                        .key("{{key}}")
                        .name("{{name}}")
                        .placeholder("user@example.com")
                        .build()?
                )
                .validation(validators::email())
                .build()?
        ),
    ])
    .build();

// Use the template
let user_email = email_template.instantiate(hashmap! {
    "key" => "user_email",
    "name" => "User Email",
});
```

### Async Validation

For validations that require external resources:

```rust
#[async_trait]
pub trait AsyncValidatable: HasValue {
    async fn validate_async(&self, value: &Self::Value) -> Result<(), ValidationError>;
}

// Example: Check username availability
impl AsyncValidatable for TextParameter {
    async fn validate_async(&self, value: &String) -> Result<(), ValidationError> {
        if self.key() != "username" {
            return Ok(());
        }

        let available = check_username_availability(value).await?;
        if !available {
            return Err(ValidationError::Custom(
                format!("Username '{}' is already taken", value)
            ));
        }

        Ok(())
    }
}
```

### Value Providers

Dynamic value computation:

```rust
#[async_trait]
pub trait ValueProvider: Send + Sync {
    async fn provide_value(&self, context: &ValueProviderContext) -> Result<Value, Error>;
}

// Example: Default from environment
pub struct EnvValueProvider {
    env_var: String,
    fallback: Option<Value>,
}

#[async_trait]
impl ValueProvider for EnvValueProvider {
    async fn provide_value(&self, _context: &ValueProviderContext) -> Result<Value, Error> {
        std::env::var(&self.env_var)
            .map(Value::String)
            .or_else(|_| self.fallback.clone().ok_or(Error::NotFound))
    }
}
```

### Parameter Migrations

Handle parameter schema changes:

```rust
pub trait ParameterMigration {
    fn migrate(&self, old_value: Value) -> Result<Value, Error>;
    fn version(&self) -> u32;
}

// Example: Migrate from string to object
pub struct EmailMigrationV2;

impl ParameterMigration for EmailMigrationV2 {
    fn migrate(&self, old_value: Value) -> Result<Value, Error> {
        match old_value {
            Value::String(email) => Ok(json!({
                "address": email,
                "verified": false,
                "primary": true,
            })),
            _ => Ok(old_value), // Already migrated
        }
    }

    fn version(&self) -> u32 {
        2
    }
}
```

## Best Practices

### 1. Use Builders for Complex Parameters

Always use builders for parameters with multiple options:

```rust
// Good
let param = TextParameter::builder()
    .metadata(/* ... */)
    .validation(/* ... */)
    .display(/* ... */)
    .build()?;

// Avoid manual construction
let param = TextParameter {
    metadata,
    value: None,
    // Easy to forget fields...
};
```

### 2. Validate at the Right Level

- Use UI options for basic constraints (min/max)
- Use validation for business rules
- Use cross-parameter validation for relationships

```rust
// UI constraint
.ui_options(NumberUiOptions::builder().min(0.0).max(100.0).build())

// Business rule
.validation(ParameterValidation::builder()
    .with_rule(ParameterCondition::custom(|value| {
        // Complex business logic
    }))
    .build())
```

### 3. Provide Clear Error Messages

Always include helpful error messages:

```rust
.validation(
    ParameterValidation::builder()
        .with_rule(ParameterCondition::custom_with_message(
            |value| validate_isbn(value),
            "Please enter a valid ISBN-10 or ISBN-13"
        ))
        .build()
)
```

### 4. Use Appropriate Parameter Types

Choose the right parameter type for the data:

- `TextParameter` for free-form text
- `SecretParameter` for sensitive data
- `SelectParameter` when choices are limited
- `NumberParameter` for numeric values with units

### 5. Group Related Parameters

Use parameter groups for better organization:

```rust
// Good: Grouped related settings
let network_group = ParameterGroup::builder()
    .metadata(GroupMetadata::new("network", "Network Settings"))
    .parameters(vec![proxy, timeout, retry_count])
    .build();

// Avoid: Flat list of unrelated parameters
```

### 6. Consider Performance

For large parameter sets:

- Use lazy loading for expensive options
- Implement caching for dynamic values
- Clear caches when parameters are hidden

```rust
impl DisplayReactive for ExpensiveParameter {
    fn on_show(&mut self, context: &DisplayContext) {
        self.load_data_if_needed(context);
    }

    fn on_hide(&mut self, _context: &DisplayContext) {
        self.clear_cache();
    }
}
```

### 7. Document Parameter Purpose

Always include clear documentation:

```rust
ParameterMetadata::builder()
    .key("retry_count")
    .name("Retry Count")
    .description("Number of times to retry failed requests")
    .hint("Set to 0 to disable retries")
    .placeholder("3")
    .build()
```

### 8. Handle Edge Cases

Consider edge cases in validation:

```rust
// Handle empty strings
.validation(ParameterValidation::builder()
    .not_empty()
    .trim_before_validation()
    .build())

// Handle number precision
.validation(ParameterValidation::builder()
    .with_rule(ParameterCondition::custom(|value: &f64| {
        if value.fract() != 0.0 && value.fract().abs() < f64::EPSILON {
            // Handle floating point precision issues
        }
        Ok(())
    }))
    .build())
```

## API Reference

### Core Traits

#### ParameterType

```rust
pub trait ParameterType {
    /// Get the kind of parameter
    fn kind(&self) -> ParameterKind;

    /// Get parameter metadata
    fn metadata(&self) -> &ParameterMetadata;

    /// Get parameter key
    fn key(&self) -> &str;

    /// Get parameter name
    fn name(&self) -> &str;

    /// Check if required
    fn is_required(&self) -> bool;
}
```

#### HasValue

```rust
pub trait HasValue: ParameterType {
    type Value: Clone + PartialEq + Debug + 'static;

    // Value access
    fn get_value(&self) -> Option<&Self::Value>;
    fn get_value_mut(&mut self) -> Option<&mut Self::Value>;
    fn has_value(&self) -> bool;

    // Value modification
    fn set_value(&mut self, value: Self::Value) -> Result<(), ParameterError>;
    fn set_value_unchecked(&mut self, value: Self::Value) -> Result<(), ParameterError>;
    fn clear_value(&mut self);
    fn take_value(&mut self) -> Option<Self::Value>;
    fn replace_value(&mut self, new: Self::Value) -> Result<Option<Self::Value>, ParameterError>;

    // Default handling
    fn default_value(&self) -> Option<&Self::Value>;
    fn is_default(&self) -> bool;
    fn reset_to_default(&mut self) -> Result<(), ParameterError>;

    // Value utilities
    fn value_or_default(&self) -> Option<&Self::Value>;
    fn value_or<'a>(&'a self, default: &'a Self::Value) -> &'a Self::Value;
    fn value_or_else<F>(&self, f: F) -> Self::Value where F: FnOnce() -> Self::Value;

    // Conversions
    fn get_parameter_value(&self) -> Option<ParameterValue>;
    fn set_parameter_value(&mut self, value: ParameterValue) -> Result<(), ParameterError>;
    fn map_value<U, F>(&self, f: F) -> Option<U> where F: FnOnce(&Self::Value) -> U;
}
```

### Error Types

```rust
#[derive(Debug, Error)]
pub enum ParameterError {
    #[error("Type mismatch for parameter '{param}': expected {expected}, got {actual}")]
    TypeMismatch {
        param: String,
        expected: &'static str,
        actual: &'static str,
    },

    #[error("Missing required parameter: {param}")]
    MissingValue { param: String },

    #[error("Validation failed for parameter '{param}': {message}")]
    ValidationFailed { param: String, message: String },

    #[error("Multiple validation errors")]
    ValidationErrors(Vec<ParameterCheckError>),

    #[error("Parse error for parameter '{param}': {source}")]
    ParseError {
        param: String,
        value: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Builder error: {0}")]
    BuilderError(String),
}
```

### Utility Functions

```rust
/// Create a parameter collection from a list of parameters
pub fn create_collection(params: Vec<Parameter>) -> ParameterCollection;

/// Validate all parameters in a collection
pub fn validate_collection(collection: &ParameterCollection) -> Result<(), Vec<ParameterError>>;

/// Extract all values from a collection
pub fn extract_values(collection: &ParameterCollection) -> HashMap<Key, ParameterValue>;

/// Apply values to a collection
pub fn apply_values(
    collection: &mut ParameterCollection,
    values: HashMap<Key, ParameterValue>
) -> Result<(), Vec<ParameterError>>;

/// Find parameter by key in a collection
pub fn find_parameter<'a>(
    collection: &'a ParameterCollection,
    key: &str
) -> Option<&'a Parameter>;

/// Get all visible parameters in a collection
pub fn visible_parameters<'a>(
    collection: &'a ParameterCollection,
    context: &DisplayContext
) -> Vec<&'a Parameter>;
```

## Examples

### Complete Node Parameter Definition

```rust
use nebula_parameter::*;

// Define parameters for an HTTP request node
pub fn create_http_request_parameters() -> ParameterCollection {
    ParameterCollection::Group(
        ParameterGroup::builder()
            .metadata(
                GroupMetadata::builder()
                    .key("http_request")
                    .name("HTTP Request")
                    .icon("globe")
                    .build()
            )
            .parameters(vec![
                // Basic settings
                ParameterCollection::Group(
                    ParameterGroup::builder()
                        .metadata(GroupMetadata::new("basic", "Basic"))
                        .parameters(vec![
                            // URL
                            ParameterCollection::Single(Parameter::Text(
                                TextParameter::builder()
                                    .metadata(
                                        ParameterMetadata::builder()
                                            .key("url")
                                            .name("URL")
                                            .required(true)
                                            .placeholder("https://api.example.com/endpoint")
                                            .build().unwrap()
                                    )
                                    .validation(validators::url())
                                    .build().unwrap()
                            )),

                            // Method
                            ParameterCollection::Single(Parameter::Select(
                                SelectParameter::builder()
                                    .metadata(
                                        ParameterMetadata::builder()
                                            .key("method")
                                            .name("Method")
                                            .required(true)
                                            .build().unwrap()
                                    )
                                    .options(vec![
                                        SelectOption::new("GET", "GET"),
                                        SelectOption::new("POST", "POST"),
                                        SelectOption::new("PUT", "PUT"),
                                        SelectOption::new("DELETE", "DELETE"),
                                        SelectOption::new("PATCH", "PATCH"),
                                    ])
                                    .default("GET".to_string())
                                    .build().unwrap()
                            )),
                        ])
                        .build()
                ),

                // Headers
                ParameterCollection::List(
                    ParameterList::builder()
                        .metadata(
                            ListMetadata::builder()
                                .key("headers")
                                .name("Headers")
                                .add_button_text("Add Header")
                                .build()
                        )
                        .item_template(create_header_parameter())
                        .build()
                ),

                // Body (conditional on method)
                ParameterCollection::Conditional(
                    ConditionalParameter::builder()
                        .condition(ParameterCondition::Or(vec![
                            ParameterCondition::Eq(json!("POST")),
                            ParameterCondition::Eq(json!("PUT")),
                            ParameterCondition::Eq(json!("PATCH")),
                        ]))
                        .then_parameters(vec![
                            ParameterCollection::Single(Parameter::Text(
                                TextParameter::builder()
                                    .metadata(
                                        ParameterMetadata::builder()
                                            .key("body")
                                            .name("Request Body")
                                            .build().unwrap()
                                    )
                                    .ui_options(TextUiOptions::multi_line(10))
                                    .validation(
                                        ParameterValidation::builder()
                                            .with_rule(ParameterCondition::custom(|value: &String| {
                                                // Validate JSON if content-type is application/json
                                                Ok(())
                                            }))
                                            .build()
                                    )
                                    .build().unwrap()
                            )),
                        ])
                        .build()
                ),

                // Advanced settings
                ParameterCollection::Group(
                    ParameterGroup::builder()
                        .metadata(GroupMetadata::new("advanced", "Advanced"))
                        .collapsible(true)
                        .default_expanded(false)
                        .parameters(vec![
                            // Timeout
                            ParameterCollection::Single(Parameter::Number(
                                NumberParameter::builder()
                                    .metadata(
                                        ParameterMetadata::builder()
                                            .key("timeout")
                                            .name("Timeout")
                                            .description("Request timeout in seconds")
                                            .build().unwrap()
                                    )
                                    .default(30.0)
                                    .ui_options(
                                        NumberUiOptions::builder()
                                            .min(1.0)
                                            .max(300.0)
                                            .step(1.0)
                                            .unit("seconds")
                                            .build()
                                    )
                                    .build().unwrap()
                            )),

                            // Retry
                            ParameterCollection::Single(Parameter::Boolean(
                                BooleanParameter::builder()
                                    .metadata(
                                        ParameterMetadata::builder()
                                            .key("retry_on_failure")
                                            .name("Retry on Failure")
                                            .build().unwrap()
                                    )
                                    .default(true)
                                    .build().unwrap()
                            )),
                        ])
                        .display(
                            ParameterDisplay::builder()
                                .show_when_equals("show_advanced", true)
                                .build()
                        )
                        .build()
                ),
            ])
            .build()
    )
}
```

This documentation provides a comprehensive guide to the nebula-parameter system, covering all major features, patterns, and best practices for building robust parameter definitions in the Nebula workflow engine.
