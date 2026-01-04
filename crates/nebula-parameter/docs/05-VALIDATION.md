# Validation

Multi-layered validation system with sync and async support.

---

## Validation Pipeline

```
User Input
    |
    v
+------------------------+
|  1. TYPE VALIDATION    |  Is value the correct type?
|     Cost: ~5ns         |
+------------------------+
    |
    v
+------------------------+
|  2. BUILT-IN           |  Min/max, length, pattern
|     CONSTRAINTS        |
|     Cost: ~50ns        |
+------------------------+
    |
    v
+------------------------+
|  3. CUSTOM SYNC        |  User-defined logic
|     VALIDATORS         |
|     Cost: ~500ns       |
+------------------------+
    |
    v
+------------------------+
|  4. CROSS-PARAMETER    |  Relationships between
|     VALIDATION         |  parameters
|     Cost: ~1us         |
+------------------------+
    |
    v
+------------------------+
|  5. ASYNC VALIDATION   |  Database, API checks
|     Cost: ~100ms       |
+------------------------+
```

---

## Validation Traits

### Sync Validator

```rust
pub trait Validator<T>: Send + Sync {
    fn validate(&self, value: &T, context: &ValidationContext) -> Result<(), ValidationError>;
}

pub struct ValidationContext<'a> {
    pub current_value: &'a Value,
    pub all_values: &'a HashMap<String, Value>,
    pub metadata: &'a Metadata,
}
```

### Async Validator

```rust
#[async_trait]
pub trait AsyncValidator<T>: Send + Sync {
    async fn validate(&self, value: &T, context: &ValidationContext) -> Result<(), ValidationError>;
}
```

---

## Validation Errors

```rust
#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("Type mismatch: expected {expected}, got {got}")]
    TypeMismatch { expected: String, got: String },
    
    #[error("Value {value} exceeds maximum {max}")]
    ExceedsMax { value: f64, max: f64 },
    
    #[error("Value {value} is below minimum {min}")]
    BelowMin { value: f64, min: f64 },
    
    #[error("Length {length} exceeds maximum {max}")]
    LengthExceedsMax { length: usize, max: usize },
    
    #[error("Length {length} is below minimum {min}")]
    LengthBelowMin { length: usize, min: usize },
    
    #[error("Value does not match pattern: {pattern}")]
    PatternMismatch { pattern: String },
    
    #[error("Required field '{field}' is missing")]
    MissingRequired { field: String },
    
    #[error("Value '{value}' is not in allowed values")]
    NotInAllowedValues { value: String },
    
    #[error("{message}")]
    Custom { message: String, fields: Vec<String> },
}
```

---

## Built-in Validators

### Required Validator

```rust
// Enabled via Flags::REQUIRED
TextParameter::builder("username")
    .required()  // Sets REQUIRED flag
    .build()
```

### Range Validator

```rust
NumberParameter::builder::<f64>("opacity")
    .hard_min(0.0)   // Fails if < 0.0
    .hard_max(1.0)   // Fails if > 1.0
    .build()
```

### Length Validator

```rust
TextParameter::builder("username")
    .min_length(3)   // Fails if < 3 chars
    .max_length(50)  // Fails if > 50 chars
    .build()
```

### Pattern Validator

```rust
TextParameter::builder("slug")
    .pattern(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
    .build()
```

### Subtype Validators

Each TextSubtype has built-in validation:

```rust
TextParameter::builder("email")
    .subtype(TextSubtype::Email)  // Validates email format
    .build()

TextParameter::builder("url")
    .subtype(TextSubtype::Url)    // Validates URL format
    .build()
```

---

## Custom Validators

### Sync Custom Validator

```rust
pub struct EvenNumberValidator;

impl Validator<i64> for EvenNumberValidator {
    fn validate(&self, value: &i64, _ctx: &ValidationContext) -> Result<(), ValidationError> {
        if value % 2 != 0 {
            return Err(ValidationError::Custom {
                message: format!("{} is not an even number", value),
                fields: vec![],
            });
        }
        Ok(())
    }
}

// Usage
NumberParameter::builder::<i64>("port")
    .validator(Arc::new(EvenNumberValidator))
    .build()
```

### Async Custom Validator

```rust
pub struct UsernameAvailabilityValidator {
    db: Arc<Database>,
}

#[async_trait]
impl AsyncValidator<String> for UsernameAvailabilityValidator {
    async fn validate(&self, value: &String, _ctx: &ValidationContext) -> Result<(), ValidationError> {
        if self.db.username_exists(value).await? {
            return Err(ValidationError::Custom {
                message: format!("Username '{}' is already taken", value),
                fields: vec!["username".into()],
            });
        }
        Ok(())
    }
}

// Usage
TextParameter::builder("username")
    .async_validator(Arc::new(UsernameAvailabilityValidator::new(db)))
    .build()
```

---

## Cross-Parameter Validation

Validate relationships between multiple parameters:

```rust
pub trait CrossValidator: Send + Sync {
    fn validate(&self, context: &Context) -> Result<(), ValidationError>;
    fn depends_on(&self) -> Vec<String>;
}

// Example: End date must be after start date
pub struct DateRangeValidator;

impl CrossValidator for DateRangeValidator {
    fn validate(&self, context: &Context) -> Result<(), ValidationError> {
        let start = context.get_datetime("start_date")?;
        let end = context.get_datetime("end_date")?;
        
        if end <= start {
            return Err(ValidationError::Custom {
                message: "End date must be after start date".into(),
                fields: vec!["start_date".into(), "end_date".into()],
            });
        }
        
        Ok(())
    }
    
    fn depends_on(&self) -> Vec<String> {
        vec!["start_date".into(), "end_date".into()]
    }
}

// Register with schema
Schema::new()
    .with(DateParameter::builder("start_date").build())
    .with(DateParameter::builder("end_date").build())
    .cross_validator(Arc::new(DateRangeValidator))
```

---

## Context Methods

```rust
impl Context {
    /// Validate single parameter (sync)
    pub fn validate(&self, key: &str) -> Result<(), Vec<ValidationError>> {
        let param = self.schema.get_parameter(key)?;
        let value = self.get_value(key)?;
        
        let mut errors = Vec::new();
        
        // Sync validation
        if let Err(e) = param.validate_sync(value) {
            errors.push(e);
        }
        
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
    
    /// Validate single parameter (async)
    pub async fn validate_async(&self, key: &str) -> Result<(), Vec<ValidationError>> {
        let param = self.schema.get_parameter(key)?;
        let value = self.get_value(key)?;
        
        // Async validation
        param.validate_async(value).await
    }
    
    /// Validate all parameters
    pub async fn validate_all(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();
        
        // Per-parameter validation
        for (key, param) in self.schema.parameters() {
            let value = self.get_value(key).unwrap_or(&Value::Null);
            
            // Sync
            if let Err(e) = param.validate_sync(value) {
                errors.push(e);
            }
            
            // Async
            if let Err(e) = param.validate_async(value).await {
                errors.push(e);
            }
        }
        
        // Cross-parameter validation
        for validator in self.schema.cross_validators() {
            if let Err(e) = validator.validate(self) {
                errors.push(e);
            }
        }
        
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
    
    /// Check if parameter is valid (cached)
    pub fn is_valid(&self, key: &str) -> bool {
        self.states.get(key)
            .map(|s| s.flags.contains(StateFlags::VALID))
            .unwrap_or(false)
    }
}
```

---

## Display Conditions

Conditionally show/hide parameters based on values:

```rust
pub enum DisplayRule {
    Show(Condition),
    Hide(Condition),
    And(Vec<DisplayRule>),
    Or(Vec<DisplayRule>),
    Not(Box<DisplayRule>),
}

pub enum Condition {
    Equals { key: String, value: Value },
    NotEquals { key: String, value: Value },
    In { key: String, values: Vec<Value> },
    GreaterThan { key: String, value: f64 },
    LessThan { key: String, value: f64 },
    Between { key: String, min: f64, max: f64 },
    IsNull { key: String },
    IsEmpty { key: String },
    IsValid { key: String },
    ModeIs { key: String, variant: String },
    Custom(Arc<dyn Fn(&Context) -> bool + Send + Sync>),
}
```

**Example:**
```rust
// Show SSL options only for HTTPS
TextParameter::builder("ssl_cert")
    .display_when(DisplayRule::And(vec![
        DisplayRule::Show(Condition::Equals {
            key: "protocol".into(),
            value: Value::Text("https".into()),
        }),
        DisplayRule::Show(Condition::Equals {
            key: "use_ssl".into(),
            value: Value::Bool(true),
        }),
    ]))
    .build()
```

---

## Validation Events

```rust
pub enum ParameterEvent {
    ValidationStarted { key: String },
    ValidationPassed { key: String },
    ValidationFailed { key: String, errors: Vec<ValidationError> },
}

// Subscribe to validation events
let mut events = context.subscribe_parameter("username");
while let Ok(event) = events.recv().await {
    match event {
        ParameterEvent::ValidationFailed { key, errors } => {
            for error in errors {
                println!("Validation error on {}: {}", key, error);
            }
        }
        _ => {}
    }
}
```

---

## State Flags

```rust
bitflags! {
    pub struct StateFlags: u32 {
        const TOUCHED    = 1 << 0;  // User interacted
        const DIRTY      = 1 << 1;  // Changed since load
        const VALIDATING = 1 << 2;  // Async validation in progress
        const VALID      = 1 << 3;  // Passed validation
        const VISIBLE    = 1 << 4;  // Currently visible (display rules)
        const ENABLED    = 1 << 5;  // Currently enabled
    }
}
```

---

## Best Practices

1. **Validate early, validate often** - Don't wait for submit
2. **Use async validators sparingly** - They're expensive
3. **Cache validation results** - Use StateFlags::VALID
4. **Provide helpful error messages** - Tell user how to fix
5. **Mark touched before validating** - Don't show errors on untouched fields
6. **Use cross-validators for relationships** - Not individual validators
