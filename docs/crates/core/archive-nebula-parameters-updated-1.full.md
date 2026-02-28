# Archived From "docs/archive/nebula_parameters_updated (1).md"

# Nebula Parameter System Documentation

## 📖 Introduction

The Nebula parameter system provides type-safe creation of configuration forms for Actions and Credentials in the workflow platform. Each parameter represents an input field with metadata, validation, and minimal UI options.

### Key Principles

- **Type Safety** - each parameter is strictly typed
- **Minimal UI Options** - only critical business settings that fundamentally change behavior
- **Platform Core Responsibility** - core handles all standard behaviors automatically
- **Clean Architecture** - parameters define data types and validation, not visual appearance
- **Unified Standards** - platform controls appearance, sizing, common behaviors
- **Composition** - complex forms from simple, focused components
- **Expression Support** - any parameter can use expressions through the core platform

### 🚨 Critical Architecture Guidelines

**DO NOT add UI options that the platform core should handle:**
- ❌ Heights, widths, sizing (`height: 10`, `cols: 80`)
- ❌ Visual styling (colors, themes, spacing)
- ❌ Standard behaviors (character counters, auto-formatting)
- ❌ Expression variables (`available_variables: vec!["$json"]`)
- ❌ Auto-completion configuration
- ❌ Preview settings (`show_preview: true`)
- ❌ Standard validation helpers (`show_schema_hints`)

**DO include UI options that change fundamental behavior:**
- ✅ Data types and constraints (`min/max`, `required`)
- ✅ Input types that change validation (`TextInputType::Email`)
- ✅ Language for syntax highlighting (`CodeLanguage::JavaScript`)
- ✅ Critical behavioral differences (`multiline: true`)
- ✅ Business logic constraints (`creatable: true` for selects)

**Platform Core Handles Automatically:**
- Expression toggle buttons and mode detection
- Variable auto-completion ($json, $node, $workflow, etc.)
- Standard sizing and responsive layout
- Color themes and visual styling
- Common behaviors (validation feedback, loading states)
- Accessibility features
- Error handling and recovery

## 💡 Expression Architecture

### ParameterValue with Expression Support

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ParameterValue {
    String(String),
    Number(f64),
    Boolean(bool),
    Array(Vec<ParameterValue>),
    Object(HashMap<String, ParameterValue>),
    DateTime(DateTime<Utc>),
    File(FileData),
    Color(String),
    
    // Expression wrapper for any value
    Expression(String),
}
```

### Expression Execution Pipeline

**Phase 1: Transform** - Convert expressions to concrete values
```rust
// Input from database
let raw_value = ParameterValue::Expression("{{$json.baseUrl}}/users");

// Transform with context
let context = ExecutionContext {
    json: previous_step_output,
    node: current_node_data,
    workflow: workflow_context,
};

let transformed = transform_expression(raw_value, &context)?;
// Result: ParameterValue::String("https://api.example.com/users")
```

**Phase 2: Validation** - Validate transformed values against parameter definitions
```rust
let validated = validate_parameters(transformed_values, &parameter_definitions)?;
```

**Phase 3: Process** - Execute action with clean values
```rust
let result = action.execute(validated_values)?;
```

### Client-Side UI Architecture

**Two-Field Approach for Maximum UX:**
```rust
// Client-side parameter input
pub struct ParameterInput {
    pub key: String,
    pub static_value: String,     // Regular input field
    pub expression_value: String, // Expression field
    pub current_mode: InputMode,  // Which field to show
}

pub enum InputMode {
    Static,
    Expression,
}
```

**Conversion: Client ↔ Database:**
```rust
// FROM database TO client (loading form)
fn parameter_value_to_input(value: &ParameterValue) -> ParameterInput {
    match value {
        ParameterValue::Expression(expr) => ParameterInput {
            static_value: String::new(),
            expression_value: expr.clone(),
            current_mode: InputMode::Expression,
        },
        ParameterValue::String(s) => ParameterInput {
            static_value: s.clone(),
            expression_value: String::new(),
            current_mode: InputMode::Static,
        },
        // ... other types
    }
}

// FROM client TO database (saving)
fn input_to_parameter_value(input: &ParameterInput, param_type: &ParameterType) -> Option<ParameterValue> {
    match input.current_mode {
        InputMode::Expression => {
            if input.expression_value.trim().is_empty() {
                None
            } else {
                Some(ParameterValue::Expression(input.expression_value.clone()))
            }
        },
        InputMode::Static => {
            // Parse according to parameter type
            match param_type {
                ParameterType::Text => Some(ParameterValue::String(input.static_value.clone())),
                ParameterType::Number => input.static_value.parse::<f64>().ok().map(ParameterValue::Number),
                // ... other types
            }
        }
    }
}
```

**Expression Mode Detection:**
```rust
fn get_input_mode(value: &Option<ParameterValue>) -> InputMode {
    match value {
        Some(ParameterValue::Expression(expr)) => {
            if is_valid_expression(expr) {
                InputMode::Expression
            } else {
                // Invalid expression → show as static with empty field
                InputMode::Static
            }
        }
        _ => InputMode::Static,
    }
}

fn is_valid_expression(expr: &str) -> bool {
    let trimmed = expr.trim();
    
    // Must contain at least one {{}} pair
    if !trimmed.contains("{{") || !trimmed.contains("}}") {
        return false;
    }
    
    // Check that all {{}} are properly paired
    let mut brace_count = 0;
    let mut chars = trimmed.chars().peekable();
    
    while let Some(c) = chars.next() {
        if c == '{' && chars.peek() == Some(&'{') {
            chars.next(); // consume second {
            brace_count += 1;
        } else if c == '}' && chars.peek() == Some(&'}') {
            chars.next(); // consume second }
            brace_count -= 1;
            if brace_count < 0 {
                return false;
            }
        }
    }
    
    brace_count == 0 // all pairs must be closed
}
```

**Platform Features (Automatic):**
- Toggle button between static/expression modes
- Auto-completion for variables ($json, $node, $workflow)
- Expression preview
- Syntax highlighting for {{}} expressions
- Error validation for malformed expressions

## 🔧 Base Components

### ParameterMetadata

Core parameter information:

```rust
pub struct ParameterMetadata {
    /// Unique parameter key
    pub key: ParameterKey,
    
    /// Display name
    pub name: Cow<'static, str>,
    
    /// Whether parameter is required
    pub required: bool,
    
    /// Parameter description
    pub description: Option<Cow<'static, str>>,
    
    /// Placeholder text for empty field
    pub placeholder: Option<Cow<'static, str>>,
    
    /// Additional information or instructions
    pub hint: Option<Cow<'static, str>>,
}
```

### Creating Metadata

```rust
// Simple creation
let metadata = ParameterMetadata::simple("api_key", "API Key")?;

// With additional information
let metadata = ParameterMetadata::builder()
    .key("timeout")
    .name("Request Timeout")
    .required(false)
    .description("Maximum time to wait for API response")
    .placeholder("30")
    .hint("Value in seconds, between 1 and 300")
    .build()?;
```

## 📝 Parameter Types

### 1. TextParameter

**Purpose:** Text input for string data (single-line and multi-line).

**When to use:**
- User names, titles
- URL addresses, email addresses
- Single-line and multi-line text
- Temporary passwords for login forms
- Long descriptions and comments

**Stored Data:** `ParameterValue::String(String)` or `ParameterValue::Expression(String)`

```rust
// Single-line text
let name = TextParameter::builder()
    .metadata(ParameterMetadata::simple("name", "User Name")?)
    .ui_options(TextUiOptions {
        input_type: TextInputType::Text,
        multiline: false,
    })
    .build()?;

// Email with validation
let email = TextParameter::builder()
    .metadata(metadata)
    .ui_options(TextUiOptions {
        input_type: TextInputType::Email,
        multiline: false,
    })
    .build()?;

// Multi-line description
let description = TextParameter::builder()
    .metadata(metadata)
    .ui_options(TextUiOptions {
        input_type: TextInputType::Text,
        multiline: true,
        rows: Some(5), // Only when critical for UX
    })
    .build()?;
```

**UI Options:**
- `input_type` - input type (Text, Password, Email, URL, Tel, Search)
- `multiline` - enable multi-line mode
- `rows` - height for multi-line (only when critical for UX)

**Note:** Most text behavior (formatting, character limits, styling) is handled by the platform core.

---

### 2. SecretParameter

**Purpose:** Secure storage of confidential data with automatic memory zeroing.

**When to use:**
- API keys and tokens
- Database passwords
- OAuth secrets
- Any long-term credentials

**Stored Data:** `ParameterValue::String(String)` (encrypted) or `ParameterValue::Expression(String)`

**Note:** Expression mode useful for dynamic secrets like `"{{$workflow.secrets.apiKey}}"` but expressions are evaluated at runtime, not stored as plain secrets.

```rust
// API key
let api_key = SecretParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("api_key")
        .name("API Key")
        .required(true)
        .description("Your service API key")
        .placeholder("sk-...")
        .build()?)
    .build()?;

// Database password
let db_password = SecretParameter::builder()
    .metadata(ParameterMetadata::required("db_password", "Database Password")?)
    .build()?;

// OAuth token (read-only)
let oauth_token = SecretParameter::builder()
    .metadata(metadata)
    .readonly(true)  // Generated automatically
    .build()?;
```

**Security:**
- Automatic memory zeroing on deletion
- Masking in logs and debug output
- Encryption during serialization
- Protection from accidental display

---

### 3. NumberParameter

**Purpose:** Numeric input with validation and formatting.

**When to use:**
- Timeouts, limits, counters
- Prices, percentages
- Performance settings
- Any numeric values

**Stored Data:** `ParameterValue::Number(f64)` or `ParameterValue::Expression(String)`

**Expression Examples:**
- Static: `30.0` → timeout in seconds
- Expression: `"{{$json.responseTime * 2}}"` → dynamic timeout calculation

```rust
// Timeout in seconds
let timeout = NumberParameter::builder()
    .metadata(metadata)
    .ui_options(NumberUiOptions {
        format: NumberFormat::Integer,
        min: Some(1.0),
        max: Some(300.0),
        step: Some(1.0),
        unit: Some("seconds".into()),
    })
    .build()?;

// Price in currency
let price = NumberParameter::builder()
    .metadata(metadata)
    .ui_options(NumberUiOptions {
        format: NumberFormat::Currency,
        min: Some(0.0),
        max: None,
        step: Some(0.01),
        unit: Some("USD".into()),
    })
    .build()?;

// Percentage (0-100)
let confidence = NumberParameter::builder()
    .metadata(metadata)
    .ui_options(NumberUiOptions {
        format: NumberFormat::Percentage,
        min: Some(0.0),
        max: Some(100.0),
        step: Some(1.0),
        unit: None,
    })
    .build()?;
```

**UI Options:**
- `format` - number format (Integer, Decimal, Currency, Percentage)
- `min/max` - value constraints
- `step` - increment step
- `unit` - unit of measurement

---

### 4. BooleanParameter

**Purpose:** Boolean values for enabling/disabling options.

**When to use:**
- Feature toggle flags
- Yes/no settings
- Terms acceptance

**Stored Data:** `ParameterValue::Boolean(bool)` or `ParameterValue::Expression(String)`

**Expression Examples:**
- Static: `true` → SSL enabled
- Expression: `"{{$json.isPremium && $json.verified}}"` → conditional logic

```rust
// Enable SSL
let use_ssl = BooleanParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("use_ssl")
        .name("Use SSL")
        .required(false)
        .description("Enable SSL encryption")
        .build()?)
    .default(true)
    .build()?;

// Terms acceptance
let accept_terms = BooleanParameter::builder()
    .metadata(ParameterMetadata::required("accept_terms", "Accept Terms")?)
    .default(false)
    .build()?;
```

---

### 5. SelectParameter

**Purpose:** Select one value from a predefined list.

**When to use:**
- HTTP methods, protocols
- Static option lists
- Categories, types
- Selection from limited set

**Stored Data:** `ParameterValue::String(String)` or `ParameterValue::Expression(String)`

**Expression Examples:**
- Static: `"POST"` → fixed HTTP method
- Expression: `"{{$json.requestType || 'GET'}}"` → dynamic method selection

```rust
// HTTP method
let method = SelectParameter::builder()
    .metadata(metadata)
    .options(vec![
        SelectOption::new("GET", "GET"),
        SelectOption::new("POST", "POST"),
        SelectOption::new("PUT", "PUT"),
        SelectOption::new("DELETE", "DELETE"),
    ])
    .ui_options(SelectUiOptions {
        searchable: false,
        creatable: false,
    })
    .build()?;

// Large list with search
let country = SelectParameter::builder()
    .metadata(metadata)
    .options(country_list)
    .ui_options(SelectUiOptions {
        searchable: true,
        creatable: false,
    })
    .build()?;

// Combobox (can add new)
let tag = SelectParameter::builder()
    .metadata(metadata)
    .options(predefined_tags)
    .ui_options(SelectUiOptions {
        searchable: true,
        creatable: true,
    })
    .build()?;
```

**UI Options:**
- `searchable` - enable search through options
- `creatable` - allow creating new values

---

### 6. MultiSelectParameter

**Purpose:** Select multiple values from a list.

**When to use:**
- Access rights, roles
- Tags, categories
- Multiple settings

**Stored Data:** `ParameterValue::Array(Vec<ParameterValue>)` or `ParameterValue::Expression(String)`

**Expression Examples:**
- Static: `["read", "write"]` → fixed permissions
- Expression: `"{{$json.user.roles}}"` → dynamic role assignment

```rust
// User permissions
let permissions = MultiSelectParameter::builder()
    .metadata(metadata)
    .options(vec![
        SelectOption::new("read", "Read Access"),
        SelectOption::new("write", "Write Access"),
        SelectOption::new("admin", "Admin Access"),
    ])
    .constraints(MultiSelectConstraints {
        min_selections: Some(1),
        max_selections: None,
    })
    .build()?;

// Article tags
let tags = MultiSelectParameter::builder()
    .metadata(metadata)
    .options(available_tags)
    .constraints(MultiSelectConstraints {
        min_selections: None,
        max_selections: Some(5),
    })
    .build()?;
```

**Constraints:**
- `min_selections` - minimum number of selections
- `max_selections` - maximum number of selections

---

### 7. RadioParameter

**Purpose:** Exclusive selection with visual radio button representation.

**When to use:**
- Authentication method selection
- Operation modes
- Mutually exclusive options

**Stored Data:** `ParameterValue::String(String)` or `ParameterValue::Expression(String)`

**Expression Examples:**
- Static: `"oauth"` → fixed auth method
- Expression: `"{{$json.hasApiKey ? 'api_key' : 'basic'}}"` → conditional auth selection

```rust
// Authentication method
let auth_method = RadioParameter::builder()
    .metadata(metadata)
    .options(vec![
        RadioOption {
            value: "basic".into(),
            label: "Basic Auth".into(),
            description: Some("Username and password".into()),
            icon: Some("user".into()),
            disabled: false,
        },
        RadioOption {
            value: "oauth".into(),
            label: "OAuth 2.0".into(),
            description: Some("OAuth authentication".into()),
            icon: Some("key".into()),
            disabled: false,
        },
    ])
    .ui_options(RadioUiOptions {
        layout: RadioLayout::Vertical,
        show_descriptions: true,
        show_icons: true,
    })
    .build()?;
```

**UI Options:**
- `layout` - layout (Vertical, Horizontal, Grid)
- `show_descriptions` - show descriptions
- `show_icons` - show icons

---

### 8. DateTimeParameter

**Purpose:** Date, time, or combined input.

**When to use:**
- Execution scheduling
- Date filtering
- Events, deadlines

**Stored Data:** `ParameterValue::DateTime(DateTime<Utc>)` or `ParameterValue::Expression(String)`

**Expression Examples:**
- Static: `"2024-12-25T09:00:00Z"` → fixed date
- Expression: `"{{$json.scheduledDate}}"` → dynamic scheduling
- Expression: `"{{new Date(Date.now() + 24*60*60*1000).toISOString()}}"` → tomorrow

```rust
// Date only
let birth_date = DateTimeParameter::builder()
    .metadata(metadata)
    .ui_options(DateTimeUiOptions {
        mode: DateTimeMode::DateOnly,
        timezone: TimezoneHandling::UTC,
        min_date: None,
        max_date: Some(today()),
    })
    .build()?;

// Date and time with timezone
let schedule = DateTimeParameter::builder()
    .metadata(metadata)
    .ui_options(DateTimeUiOptions {
        mode: DateTimeMode::DateTime,
        timezone: TimezoneHandling::UserLocal,
        min_date: Some(today()),
        max_date: None,
    })
    .build()?;
```

**UI Options:**
- `mode` - mode (DateOnly, TimeOnly, DateTime)
- `timezone` - timezone handling (UTC, UserLocal, Custom)
- `min_date/max_date` - date constraints

---

### 9. CodeParameter

**Purpose:** Code editor with syntax highlighting and auto-completion.

**When to use:**
- JavaScript expressions
- SQL queries
- JSON templates
- HTML/CSS code

**Stored Data:** `ParameterValue::String(String)` or `ParameterValue::Expression(String)`

**Note:** CodeParameter typically stores static code, but can use expressions for dynamic code generation:
- Static: `"SELECT * FROM users WHERE active = true"`
- Expression: `"{{$json.customQuery || 'SELECT * FROM users'}}"`

```rust
// JavaScript expression
let expression = CodeParameter::builder()
    .metadata(metadata)
    .ui_options(CodeUiOptions {
        language: CodeLanguage::JavaScript,
        height: 6,
        available_variables: vec![
            "$json".into(),
            "$node".into(),
            "$workflow".into(),
        ],
    })
    .build()?;

// SQL query
let query = CodeParameter::builder()
    .metadata(metadata)
    .ui_options(CodeUiOptions {
        language: CodeLanguage::SQL,
        height: 10,
        available_variables: vec![
            "$input".into(),
            "$params".into(),
        ],
    })
    .build()?;
```

**UI Options:**
- `language` - programming language
- `height` - editor height in lines
- `available_variables` - variables for auto-completion

---

### 10. ExpressionParameter

**Purpose:** Expression input with {{}} syntax support.

**When to use:**
- Dynamic values
- Text templates
- Computed fields

```rust
// Email subject with variables
let subject = ExpressionParameter::builder()
    .metadata(metadata)
    .ui_options(ExpressionUiOptions {
        mode: ExpressionMode::Mixed,
        available_variables: vec![
            ExpressionVariable {
                name: "User Name".into(),
                path: "$json.user.name".into(),
                description: "Name of the user".into(),
                example_value: json!("John Doe"),
            }
        ],
        show_preview: true,
        highlight_expressions: true,
    })
    .build()?;
```

---

### 11. ResourceParameter

**Purpose:** Universal SDK for dynamic resource loading from external APIs.

**When to use:**
- Slack channels, users
- Database tables
- Google Drive folders
- GitHub repositories
- File systems
- Any external data source

**Stored Data:** `ParameterValue::String(String)` (resource ID) or `ParameterValue::Expression(String)`

**Expression Examples:**
- Static: `"C1234567890"` → fixed Slack channel ID
- Expression: `"{{$json.targetChannel}}"` → dynamic channel selection

#### Universal Architecture

```rust
pub struct ResourceParameter {
    pub metadata: ParameterMetadata,
    
    /// Universal resource loader
    pub loader: ResourceLoader,
    
    /// Cache configuration
    pub cache_config: CacheConfig,
    
    /// UI configuration
    pub ui_config: ResourceUIConfig,
    
    /// Error handling
    pub error_handling: ErrorHandling,
}

/// Universal loader - building block for any resource
pub struct ResourceLoader {
    /// Data loading function
    pub load_fn: LoadFunction,
    
    /// Dependencies on other parameters
    pub dependencies: Vec<String>,
    
    /// Loading strategy
    pub loading_strategy: LoadingStrategy,
    
    /// Validation of loaded data
    pub validation: Option<ValidationFunction>,
    
    /// Data transformation before display
    pub transform: Option<TransformFunction>,
}

/// Flexible loading function
pub type LoadFunction = Box<dyn Fn(LoadContext) -> BoxFuture<'static, Result<Vec<ResourceItem>, LoadError>>>;

/// Loading context - everything the load function needs to know
pub struct LoadContext {
    /// Values of dependent parameters
    pub dependencies: HashMap<String, ParameterValue>,
    
    /// Credentials for authentication
    pub credentials: HashMap<String, Credential>,
    
    /// Additional parameters (filters, pagination, etc)
    pub params: HashMap<String, serde_json::Value>,
    
    /// User context
    pub user_context: UserContext,
    
    /// HTTP client
    pub http_client: HttpClient,
    
    /// Cache for reading previous results
    pub cache: CacheReader,
}

/// Universal resource item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceItem {
    /// Unique ID for selection
    pub id: String,
    
    /// Display text
    pub label: String,
    
    /// Optional description
    pub description: Option<String>,
    
    /// Optional icon/image
    pub icon: Option<ResourceIcon>,
    
    /// Arbitrary metadata
    pub metadata: serde_json::Map<String, serde_json::Value>,
    
    /// Available for selection
    pub enabled: bool,
    
    /// Group for UI grouping
    pub group: Option<String>,
    
    /// Sort key
    pub sort_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResourceIcon {
    /// Image URL
    Url(String),
    
    /// Icon name from icon set
    Icon(String),
    
    /// Emoji
    Emoji(String),
    
    /// Base64 encoded image
    Data {
        mime_type: String,
        data: String,
    },
}
```

#### Loading Strategies

```rust
#[derive(Debug, Clone)]
pub enum LoadingStrategy {
    /// Load immediately when dependencies are ready
    Immediate,
    
    /// Load on field focus
    OnFocus,
    
    /// Load when dropdown opens
    OnDemand,
    
    /// Progressive loading with pagination
    Progressive {
        page_size: usize,
        load_more_threshold: usize,
    },
    
    /// Custom strategy
    Custom(Box<dyn Fn(&LoadContext) -> bool>),
}

/// Cache configuration
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Cache enabled
    pub enabled: bool,
    
    /// TTL for cached data
    pub ttl: Duration,
    
    /// Cache key (function of context)
    pub cache_key_fn: CacheKeyFunction,
    
    /// Invalidation strategy
    pub invalidation: InvalidationStrategy,
    
    /// Cache storage
    pub storage: CacheStorage,
}

pub type CacheKeyFunction = Box<dyn Fn(&LoadContext) -> String>;

#[derive(Debug, Clone)]
pub enum InvalidationStrategy {
    /// TTL only
    TimeOnly,
    
    /// On any dependency change
    OnDependencyChange,
    
    /// On specific dependency changes
    OnSpecificDependencies(Vec<String>),
    
    /// Custom logic
    Custom(Box<dyn Fn(&LoadContext, &LoadContext) -> bool>),
}

#[derive(Debug, Clone)]
pub enum CacheStorage {
    /// In memory (while tab is open)
    Memory,
    
    /// In localStorage
    LocalStorage { prefix: String },
    
    /// In IndexedDB
    IndexedDB { 
        database: String, 
        table: String 
    },
    
    /// No caching
    None,
}
```

#### Builder API for Developers

```rust
impl ResourceParameter {
    /// Create simple HTTP resource
    pub fn http_resource(url: &str) -> ResourceParameterBuilder {
        ResourceParameterBuilder::new()
            .load_from_url(url)
    }
    
    /// Create resource with dependencies
    pub fn dependent_resource() -> ResourceParameterBuilder {
        ResourceParameterBuilder::new()
    }
    
    /// Create fully custom resource
    pub fn custom_resource<F>(load_fn: F) -> ResourceParameterBuilder 
    where 
        F: Fn(LoadContext) -> BoxFuture<'static, Result<Vec<ResourceItem>, LoadError>> + 'static
    {
        ResourceParameterBuilder::new()
            .loader(LoadFunction::new(load_fn))
    }
}

/// Convenient builder for configuration
pub struct ResourceParameterBuilder {
    metadata: Option<ParameterMetadata>,
    loader: Option<ResourceLoader>,
    cache_config: CacheConfig,
    ui_config: ResourceUIConfig,
    error_handling: ErrorHandling,
}

impl ResourceParameterBuilder {
    /// HTTP loading with URL template
    pub fn load_from_url(mut self, url: &str) -> Self {
        self.loader = Some(ResourceLoader {
            load_fn: Box::new(move |ctx| {
                let resolved_url = resolve_template(url, &ctx);
                Box::pin(async move {
                    let response = ctx.http_client.get(&resolved_url).send().await?;
                    let items: Vec<ResourceItem> = response.json().await?;
                    Ok(items)
                })
            }),
            dependencies: extract_dependencies_from_template(url),
            loading_strategy: LoadingStrategy::OnDemand,
            validation: None,
            transform: None,
        });
        self
    }
    
    /// Dependencies on other parameters
    pub fn depends_on(mut self, dependencies: Vec<&str>) -> Self {
        if let Some(ref mut loader) = self.loader {
            loader.dependencies = dependencies.into_iter().map(String::from).collect();
        }
        self
    }
    
    /// Custom loading function
    pub fn load_with<F>(mut self, load_fn: F) -> Self 
    where 
        F: Fn(LoadContext) -> BoxFuture<'static, Result<Vec<ResourceItem>, LoadError>> + 'static
    {
        if let Some(ref mut loader) = self.loader {
            loader.load_fn = Box::new(load_fn);
        }
        self
    }
    
    /// Transform loaded data
    pub fn transform<F>(mut self, transform_fn: F) -> Self 
    where 
        F: Fn(Vec<ResourceItem>) -> Vec<ResourceItem> + 'static
    {
        if let Some(ref mut loader) = self.loader {
            loader.transform = Some(Box::new(transform_fn));
        }
        self
    }
    
    /// Data validation
    pub fn validate<F>(mut self, validate_fn: F) -> Self 
    where 
        F: Fn(&[ResourceItem]) -> Result<(), String> + 'static
    {
        if let Some(ref mut loader) = self.loader {
            loader.validation = Some(Box::new(validate_fn));
        }
        self
    }
    
    /// Cache configuration
    pub fn cache(mut self, ttl: Duration) -> Self {
        self.cache_config.enabled = true;
        self.cache_config.ttl = ttl;
        self
    }
    
    /// Custom cache key
    pub fn cache_key<F>(mut self, key_fn: F) -> Self 
    where 
        F: Fn(&LoadContext) -> String + 'static
    {
        self.cache_config.cache_key_fn = Box::new(key_fn);
        self
    }
    
    /// Loading strategy
    pub fn loading_strategy(mut self, strategy: LoadingStrategy) -> Self {
        if let Some(ref mut loader) = self.loader {
            loader.loading_strategy = strategy;
        }
        self
    }
    
    /// Final build
    pub fn build(self) -> Result<ResourceParameter, ParameterError> {
        Ok(ResourceParameter {
            metadata: self.metadata.ok_or(ParameterError::MissingMetadata)?,
            loader: self.loader.ok_or(ParameterError::MissingLoader)?,
            cache_config: self.cache_config,
            ui_config: self.ui_config,
            error_handling: self.error_handling,
        })
    }
}
```

#### Usage Examples

**Simple HTTP Resource:**
```rust
// Developer creates parameter for user selection
let users_param = ResourceParameter::http_resource("https://api.example.com/users")
    .metadata(ParameterMetadata::required("user_id", "User")?)
    .cache(Duration::minutes(10))
    .transform(|mut items| {
        // Sort by name
        items.sort_by(|a, b| a.label.cmp(&b.label));
        items
    })
    .build()?;
```

**Resource with Dependencies:**
```rust
// Slack channels depend on workspace
let channels_param = ResourceParameter::dependent_resource()
    .metadata(ParameterMetadata::required("channel_id", "Slack Channel")?)
    .depends_on(vec!["workspace_id", "credential"])
    .load_with(|ctx| Box::pin(async move {
        let workspace_id = ctx.dependencies.get("workspace_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| LoadError::MissingDependency("workspace_id".into()))?;
            
        let credential = ctx.credentials.get("slack_oauth2")
            .ok_or_else(|| LoadError::MissingCredential("slack_oauth2".into()))?;
        
        let response = ctx.http_client
            .get("https://slack.com/api/conversations.list")
            .header("Authorization", format!("Bearer {}", credential.token))
            .query(&[("types", "public_channel,private_channel")])
            .send()
            .await?;
            
        let data: serde_json::Value = response.json().await?;
        let channels = data["channels"].as_array()
            .ok_or_else(|| LoadError::InvalidResponse("Missing channels array".into()))?;
        
        let mut items = Vec::new();
        for channel in channels {
            if let Some(id) = channel["id"].as_str() {
                let name = channel["name"].as_str().unwrap_or("Unknown");
                let is_private = channel["is_private"].as_bool().unwrap_or(false);
                
                items.push(ResourceItem {
                    id: id.to_string(),
                    label: format!("#{}", name),
                    description: channel["purpose"]["value"].as_str().map(String::from),
                    icon: Some(ResourceIcon::Icon(
                        if is_private { "lock" } else { "hash" }.to_string()
                    )),
                    metadata: {
                        let mut map = serde_json::Map::new();
                        map.insert("is_private".into(), json!(is_private));
                        map.insert("name".into(), json!(name));
                        map
                    },
                    enabled: true,
                    group: Some(if is_private { "Private" } else { "Public" }.to_string()),
                    sort_key: Some(name.to_lowercase()),
                });
            }
        }
        
        Ok(items)
    }))
    .cache_key(|ctx| {
        format!("slack_channels_{}", 
            ctx.dependencies.get("workspace_id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
        )
    })
    .cache(Duration::minutes(10))
    .loading_strategy(LoadingStrategy::OnDemand)
    .build()?;
```

**Database Tables:**
```rust
// Database tables
let tables_param = ResourceParameter::dependent_resource()
    .metadata(ParameterMetadata::required("table_name", "Database Table")?)
    .depends_on(vec!["connection"])
    .load_with(|ctx| Box::pin(async move {
        let connection = ctx.credentials.get("database")
            .ok_or_else(|| LoadError::MissingCredential("database".into()))?;
        
        // Database connection through connection string
        let db_url = format!("postgresql://{}:{}@{}:{}/{}",
            connection.username,
            connection.password,
            connection.host,
            connection.port,
            connection.database
        );
        
        let response = ctx.http_client
            .post("/api/database/query")
            .json(&json!({
                "connection": db_url,
                "query": "SELECT schemaname, tablename, tableowner FROM pg_tables ORDER BY schemaname, tablename"
            }))
            .send()
            .await?;
            
        let rows: Vec<serde_json::Value> = response.json().await?;
        
        let mut items = Vec::new();
        for row in rows {
            let schema = row["schemaname"].as_str().unwrap_or("public");
            let table = row["tablename"].as_str().unwrap_or("unknown");
            let owner = row["tableowner"].as_str().unwrap_or("unknown");
            
            items.push(ResourceItem {
                id: format!("{}.{}", schema, table),
                label: table.to_string(),
                description: Some(format!("Owner: {}", owner)),
                icon: Some(ResourceIcon::Icon("table".to_string())),
                metadata: {
                    let mut map = serde_json::Map::new();
                    map.insert("schema".into(), json!(schema));
                    map.insert("owner".into(), json!(owner));
                    map
                },
                enabled: true,
                group: Some(schema.to_string()),
                sort_key: Some(format!("{}_{}", schema, table)),
            });
        }
        
        Ok(items)
    }))
    .validate(|items| {
        if items.is_empty() {
            Err("No tables found. Check your database connection.".to_string())
        } else {
            Ok(())
        }
    })
    .cache(Duration::hours(1))
    .build()?;
```

**Key Benefits:**
- **Complete Flexibility** - developers can create ANY resource
- **Simplicity for Simple Cases** - simple HTTP resource in one line
- **Power for Complex Cases** - full control over the entire process
- **Composability** - helper functions can be created
- **No Vendor Lock-in** - no hardcoded resource types

---

### 12. FileParameter

**Purpose:** File upload and selection.

**When to use:**
- Document upload
- User avatars
- CSV files for processing

**Stored Data:** `ParameterValue::File(FileData)` or `ParameterValue::Expression(String)`

**Expression Examples:**
- Static: File uploaded directly
- Expression: `"{{$json.attachmentUrl}}"` → dynamic file reference

```rust
// CSV file
let csv_file = FileParameter::builder()
    .metadata(metadata)
    .ui_options(FileUiOptions {
        accept: vec!["text/csv".into(), ".csv".into()],
        max_size: Some(10 * 1024 * 1024), // 10MB
        multiple: false,
        preview: false,
    })
    .build()?;

// Avatar image
let avatar = FileParameter::builder()
    .metadata(metadata)
    .ui_options(FileUiOptions {
        accept: vec!["image/*".into()],
        max_size: Some(2 * 1024 * 1024), // 2MB
        multiple: false,
        preview: true,
    })
    .build()?;
```

---

### 13. ColorParameter

**Purpose:** Color selection with various format support.

**When to use:**
- Slack message colors
- Interface themes
- Appearance settings

**Stored Data:** `ParameterValue::String(String)` (hex/rgb) or `ParameterValue::Expression(String)`

**Expression Examples:**
- Static: `"#36a64f"` → fixed green color
- Expression: `"{{$json.status === 'error' ? '#ff0000' : '#36a64f'}}"` → conditional coloring

```rust
// Message color
let message_color = ColorParameter::builder()
    .metadata(metadata)
    .ui_options(ColorUiOptions {
        format: ColorFormat::Hex,
        palette: vec![
            "#36a64f".into(), // green
            "#ff0000".into(), // red
            "#ffaa00".into(), // orange
        ],
        alpha: false,
    })
    .build()?;
```

---



### 14. HiddenParameter

**Purpose:** Hidden parameters for internal needs.

**When to use:**
- Internal identifiers
- Workflow state
- System parameters

**Stored Data:** Any `ParameterValue` type or `ParameterValue::Expression(String)`

**Note:** Hidden parameters support expressions for dynamic internal values like `"{{$workflow.instanceId}}"` but are not visible in UI.

```rust
// Internal ID
let internal_id = HiddenParameter::builder()
    .metadata(metadata)
    .value(Some(ParameterValue::String("workflow_123".into())))
    .build()?;
```

---

### 15. NoticeParameter

**Purpose:** Display informational messages.

**When to use:**
- API limit warnings
- Setup instructions
- Status information

**Stored Data:** No stored value (display-only parameter)

**Note:** Notice parameters are for UI messaging and don't store data. They may use expressions in their content for dynamic messages.

```rust
// Limit warning
let warning = NoticeParameter::builder()
    .metadata(metadata)
    .notice_type(NoticeType::Warning)
    .ui_options(NoticeUiOptions {
        dismissible: true,
        show_icon: true,
        markdown: true,
    })
    .build()?;
```

---

### 16. CheckboxParameter

**Purpose:** Checkbox for boolean values with special UI representation.

**When to use:**
- Terms acceptance
- Multiple independent options
- When checkbox UI is needed instead of toggle

**Stored Data:** `ParameterValue::Boolean(bool)` or `ParameterValue::Expression(String)`

**Expression Examples:**
- Static: `true` → terms accepted
- Expression: `"{{$json.user.hasConsent}}"` → dynamic consent check

```rust
// Terms acceptance
let accept_terms = CheckboxParameter::builder()
    .metadata(ParameterMetadata::required("accept_terms", "Accept Terms and Conditions")?)
    .ui_options(CheckboxUiOptions {
        label_position: LabelPosition::Right,
        show_description: true,
        indeterminate: false,
    })
    .build()?;

// Email notifications
let email_notifications = CheckboxParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("email_notifications")
        .name("Email Notifications")
        .description("Receive notifications via email")
        .build()?)
    .default(true)
    .build()?;
```

**UI Options:**
- `label_position` - text position relative to checkbox
- `show_description` - show description
- `indeterminate` - third state support

---

### 17. TextareaParameter

**Purpose:** Specialized multi-line text input.

**When to use:**
- Long descriptions
- Comments
- Multi-line configurations
- When textarea-specific options are needed

**Stored Data:** `ParameterValue::String(String)` or `ParameterValue::Expression(String)`

**Expression Examples:**
- Static: `"Project description here..."`
- Expression: `"{{$json.title}}\n\n{{$json.description}}"` → dynamic content formatting

```rust
// Project description
let project_description = TextareaParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("description")
        .name("Project Description")
        .description("Detailed description of the project")
        .placeholder("Enter project description...")
        .build()?)
    .ui_options(TextareaUiOptions {
        rows: 8,
        cols: None,
        resize: ResizeMode::Vertical,
        wrap: WrapMode::Soft,
        show_counter: true,
    })
    .build()?;

// SQL comment
let sql_comment = TextareaParameter::builder()
    .metadata(metadata)
    .ui_options(TextareaUiOptions {
        rows: 4,
        cols: Some(80),
        resize: ResizeMode::Both,
        wrap: WrapMode::Hard,
        show_counter: false,
    })
    .build()?;
```

**UI Options:**
- `rows` - number of visible lines
- `cols` - width in characters
- `resize` - resize mode (None, Both, Horizontal, Vertical)
- `wrap` - line wrap mode (Soft, Hard, Off)
- `show_counter` - show character counter

---

### 18. DateParameter

**Purpose:** Date-only input without time.

**When to use:**
- Date of birth
- Deadlines
- Event dates without time binding

**Stored Data:** `ParameterValue::DateTime(DateTime<Utc>)` (time set to 00:00:00) or `ParameterValue::Expression(String)`

**Expression Examples:**
- Static: `"2024-12-25"` → Christmas date
- Expression: `"{{$json.deadline}}"` → dynamic deadline

```rust
// Date of birth
let birth_date = DateParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("birth_date")
        .name("Date of Birth")
        .build()?)
    .ui_options(DateUiOptions {
        format: Some("YYYY-MM-DD".into()),
        min_date: Some(NaiveDate::from_ymd_opt(1900, 1, 1).unwrap()),
        max_date: Some(today()),
        show_week_numbers: false,
    })
    .build()?;

// Project deadline
let deadline = DateParameter::builder()
    .metadata(metadata)
    .ui_options(DateUiOptions {
        format: None, // Local format
        min_date: Some(today()),
        max_date: None,
        show_week_numbers: true,
    })
    .build()?;
```

**UI Options:**
- `format` - date display format
- `min_date/max_date` - date constraints
- `show_week_numbers` - show week numbers

---

### 19. TimeParameter

**Purpose:** Time-only input without date.

**When to use:**
- Meeting time
- Working hours
- Schedules

**Stored Data:** `ParameterValue::String(String)` (time format like "09:30:00") or `ParameterValue::Expression(String)`

**Expression Examples:**
- Static: `"14:30:00"` → 2:30 PM
- Expression: `"{{$json.meetingTime}}"` → dynamic time scheduling

```rust
// Meeting time
let meeting_time = TimeParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("meeting_time")
        .name("Meeting Time")
        .build()?)
    .ui_options(TimeUiOptions {
        format: TimeFormat::Hour24,
        step: Some(Duration::minutes(15)),
        show_seconds: false,
    })
    .build()?;

// Time with seconds
let precise_time = TimeParameter::builder()
    .metadata(metadata)
    .ui_options(TimeUiOptions {
        format: TimeFormat::Hour12,
        step: Some(Duration::seconds(1)),
        show_seconds: true,
    })
    .build()?;
```

**UI Options:**
- `format` - time format (12/24 hour)
- `step` - time change step
- `show_seconds` - show seconds

---

## 🗂️ Container Parameters

### 20. GroupParameter

**Purpose:** Visual grouping of related parameters.

**When to use:**
- Settings sections
- Logical parameter groups
- Collapsible panels

**Stored Data:** `ParameterValue::Object(HashMap<String, ParameterValue>)` containing all child parameter values

**Note:** Group parameters aggregate their children's values. Expressions can be used in individual child parameters.

```rust
// Database settings group
let db_group = GroupParameter::builder()
    .metadata(GroupMetadata::builder()
        .key("database")
        .name("Database Settings")
        .description("Configure database connection")
        .build()?)
    .parameters(vec![
        Parameter::Text(host),
        Parameter::Number(port),
        Parameter::Text(username),
        Parameter::Secret(password),
    ])
    .ui_options(GroupUiOptions {
        collapsible: true,
        default_expanded: false,
        layout: GroupLayout::Vertical,
    })
    .build()?;
```

---

### 21. ObjectParameter

**Purpose:** Structured data container with fixed named fields that form a cohesive logical unit.

**When to use:**
- HTTP headers (always `name` + `value`)
- Database connections (always `host` + `port` + `username` + `password`)
- API endpoints (always `method` + `url` + `headers`)
- Coordinates (always `x` + `y` + optional `z`)
- Complex configurations with interdependent fields

**When NOT to use:**
- Different data types (use ModeParameter instead)
- UI grouping only (use GroupParameter instead)
- Dynamic field structure (use ResourceParameter with object response)

**Stored Data:** `ParameterValue::Object(HashMap<String, ParameterValue>)` or `ParameterValue::Expression(String)`

**Expression Examples:**
- Static: `{"name": "Content-Type", "value": "application/json"}`
- Expression: `"{{JSON.stringify({Authorization: 'Bearer ' + $json.token})}}"` → dynamic header

#### Architecture Principles

**🎯 Core Principle: Fixed Named Fields**
- Each field has a specific name and type
- Fields are defined at creation time, not runtime
- All fields together form a single logical unit
- Cross-field validation and dependencies

**🔧 Key Characteristics:**
- **Semantic Cohesion** - fields are meaningfully related
- **Fixed Structure** - cannot add/remove fields dynamically
- **Type Safety** - each field is strictly typed
- **Cross-Field Validation** - fields can be validated together

#### Core Structure

```rust
pub struct ObjectParameter {
    pub metadata: ParameterMetadata,
    
    /// Fixed fields of the object
    pub fields: IndexMap<String, Parameter>,
    
    /// UI configuration
    pub ui_config: ObjectUIConfig,
    
    /// Object-level validation
    pub validation: Option<ObjectValidation>,
}

pub struct ObjectUIConfig {
    /// Field layout
    pub layout: ObjectLayout,
    
    /// Show field labels
    pub show_labels: bool,
    
    /// Compact mode
    pub compact: bool,
    
    /// Field grouping
    pub field_groups: Vec<FieldGroup>,
}

pub enum ObjectLayout {
    /// Fields vertically
    Vertical,
    
    /// Fields horizontally
    Horizontal,
    
    /// Grid layout
    Grid { columns: usize },
    
    /// Custom grid template
    CustomGrid { template: String },
}

pub struct FieldGroup {
    pub name: String,
    pub fields: Vec<String>,
    pub collapsible: bool,
}

pub type ObjectValidation = Box<dyn Fn(&HashMap<String, ParameterValue>) -> Result<(), String>>;
```

#### Builder API

```rust
impl ObjectParameter {
    /// Create builder
    pub fn builder() -> ObjectParameterBuilder {
        ObjectParameterBuilder::new()
    }
    
    /// Quick creation with fields
    pub fn with_fields(fields: Vec<(&str, Parameter)>) -> ObjectParameterBuilder {
        let mut builder = ObjectParameterBuilder::new();
        for (name, param) in fields {
            builder = builder.add_field(name, param);
        }
        builder
    }
}

impl ObjectParameterBuilder {
    /// Add field to object
    pub fn add_field<S: Into<String>>(mut self, name: S, param: Parameter) -> Self {
        self.fields.insert(name.into(), param);
        self
    }
    
    /// Add multiple fields at once
    pub fn add_fields(mut self, fields: Vec<(&str, Parameter)>) -> Self {
        for (name, param) in fields {
            self.fields.insert(name.into(), param);
        }
        self
    }
    
    /// Conditionally add field
    pub fn add_field_if<S: Into<String>>(
        mut self, 
        condition: bool, 
        name: S, 
        param: Parameter
    ) -> Self {
        if condition {
            self.fields.insert(name.into(), param);
        }
        self
    }
    
    /// Short alias for add_field
    pub fn field(self, name: &str, param: Parameter) -> Self {
        self.add_field(name, param)
    }
    
    /// Set layout
    pub fn layout(mut self, layout: ObjectLayout) -> Self {
        self.ui_config.layout = layout;
        self
    }
    
    /// Group fields visually
    pub fn field_group(mut self, name: &str, fields: Vec<&str>) -> Self {
        self.ui_config.field_groups.push(FieldGroup {
            name: name.to_string(),
            fields: fields.into_iter().map(String::from).collect(),
            collapsible: false,
        });
        self
    }
    
    /// Add object-level validation
    pub fn validate<F>(mut self, validate_fn: F) -> Self 
    where 
        F: Fn(&HashMap<String, ParameterValue>) -> Result<(), String> + 'static
    {
        self.validation = Some(Box::new(validate_fn));
        self
    }
    
    /// Build the parameter
    pub fn build(self) -> Result<ObjectParameter, ParameterError> {
        if self.fields.is_empty() {
            return Err(ParameterError::EmptyObject);
        }
        
        Ok(ObjectParameter {
            metadata: self.metadata.ok_or(ParameterError::MissingMetadata)?,
            fields: self.fields,
            ui_config: self.ui_config,
            validation: self.validation,
        })
    }
}
```

#### Usage Examples

**Simple HTTP Header:**
```rust
let http_header = ObjectParameter::builder()
    .metadata(ParameterMetadata::simple("header", "HTTP Header")?)
    .add_field("name", TextParameter::builder()
        .metadata(ParameterMetadata::required("name", "Header Name")?)
        .build()?)
    .add_field("value", TextParameter::builder()
        .metadata(ParameterMetadata::required("value", "Header Value")?)
        .build()?)
    .layout(ObjectLayout::Horizontal)
    .validate(|fields| {
        let name = fields.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let value = fields.get("value").and_then(|v| v.as_str()).unwrap_or("");
        
        if name.is_empty() {
            return Err("Header name is required".to_string());
        }
        
        if name.contains(" ") {
            return Err("Header name cannot contain spaces".to_string());
        }
        
        if value.is_empty() {
            return Err("Header value is required".to_string());
        }
        
        Ok(())
    })
    .build()?;
```

**Database Connection with Field Groups:**
```rust
let db_connection = ObjectParameter::builder()
    .metadata(ParameterMetadata::required("connection", "Database Connection")?)
    .add_field("host", TextParameter::builder()
        .metadata(ParameterMetadata::builder()
            .key("host")
            .name("Host")
            .required(true)
            .placeholder("localhost")
            .build()?)
        .build()?)
    .add_field("port", NumberParameter::builder()
        .metadata(ParameterMetadata::builder()
            .key("port")
            .name("Port")
            .required(true)
            .build()?)
        .ui_options(NumberUiOptions {
            format: NumberFormat::Integer,
            min: Some(1.0),
            max: Some(65535.0),
            step: Some(1.0),
            unit: None,
        })
        .default(5432.0)
        .build()?)
    .add_field("database", TextParameter::builder()
        .metadata(ParameterMetadata::required("database", "Database Name")?)
        .build()?)
    .add_field("username", TextParameter::builder()
        .metadata(ParameterMetadata::required("username", "Username")?)
        .build()?)
    .add_field("password", SecretParameter::builder()
        .metadata(ParameterMetadata::required("password", "Password")?)
        .build()?)
    .layout(ObjectLayout::Grid { columns: 2 })
    .field_group("Connection", vec!["host", "port", "database"])
    .field_group("Authentication", vec!["username", "password"])
    .validate(|fields| {
        let host = fields.get("host").and_then(|v| v.as_str()).unwrap_or("");
        let port = fields.get("port").and_then(|v| v.as_f64()).unwrap_or(0.0);
        
        if host.is_empty() {
            return Err("Host is required".to_string());
        }
        
        if port < 1.0 || port > 65535.0 {
            return Err("Port must be between 1 and 65535".to_string());
        }
        
        // Cross-field validation
        if host == "localhost" && port != 5432.0 {
            return Err("For localhost, please use default port 5432".to_string());
        }
        
        Ok(())
    })
    .build()?;
```

**API Endpoint with Conditional Fields:**
```rust
let api_endpoint = ObjectParameter::builder()
    .metadata(ParameterMetadata::required("endpoint", "API Endpoint")?)
    .add_field("method", SelectParameter::builder()
        .metadata(ParameterMetadata::required("method", "HTTP Method")?)
        .options(vec![
            SelectOption::new("GET", "GET"),
            SelectOption::new("POST", "POST"),
            SelectOption::new("PUT", "PUT"),
            SelectOption::new("DELETE", "DELETE"),
        ])
        .build()?)
    .add_field("url", TextParameter::builder()
        .metadata(ParameterMetadata::required("url", "URL")?)
        .ui_options(TextUiOptions {
            input_type: TextInputType::URL,
            multiline: false,
        })
        .build()?)
    .add_field("headers", ListParameter::new(
        ObjectParameter::builder()
            .metadata(ParameterMetadata::simple("header", "Header")?)
            .add_field("name", text_parameter!("name", "Name"))
            .add_field("value", text_parameter!("value", "Value"))
            .layout(ObjectLayout::Horizontal)
            .build()?
    ).build()?)
    .add_field("body", TextParameter::builder()
        .metadata(ParameterMetadata::builder()
            .key("body")
            .name("Request Body")
            .required(false)
            .build()?)
        .ui_options(TextUiOptions {
            input_type: TextInputType::Text,
            multiline: true,
            rows: Some(6),
        })
        .display(ParameterDisplay::builder()
            .show_when("method", ParameterCondition::Or(vec![
                ParameterCondition::Eq(json!("POST")),
                ParameterCondition::Eq(json!("PUT")),
                ParameterCondition::Eq(json!("PATCH")),
            ]))
            .build())
        .build()?)
    .layout(ObjectLayout::Vertical)
    .validate(|fields| {
        let method = fields.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let url = fields.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let body = fields.get("body").and_then(|v| v.as_str()).unwrap_or("");
        
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err("URL must start with http:// or https://".to_string());
        }
        
        if ["POST", "PUT", "PATCH"].contains(&method) && body.is_empty() {
            return Err(format!("{} requests typically require a body", method));
        }
        
        Ok(())
    })
    .build()?;
```

**Complex Telegram Button:**
```rust
let telegram_button = ObjectParameter::builder()
    .metadata(ParameterMetadata::required("button", "Telegram Button")?)
    .add_field("text", TextParameter::builder()
        .metadata(ParameterMetadata::required("text", "Button Text")?)
        .build()?)
    .add_field("type", SelectParameter::builder()
        .metadata(ParameterMetadata::required("type", "Button Type")?)
        .options(vec![
            SelectOption::new("url", "URL"),
            SelectOption::new("callback_data", "Callback Data"),
            SelectOption::new("switch_inline_query", "Switch Inline Query"),
            SelectOption::new("web_app", "Web App"),
        ])
        .build()?)
    .add_field("url", TextParameter::builder()
        .metadata(ParameterMetadata::builder()
            .key("url")
            .name("URL")
            .required(false)
            .build()?)
        .ui_options(TextUiOptions {
            input_type: TextInputType::URL,
            multiline: false,
        })
        .display(ParameterDisplay::builder()
            .show_when("type", ParameterCondition::Eq(json!("url")))
            .build())
        .build()?)
    .add_field("callback_data", TextParameter::builder()
        .metadata(ParameterMetadata::builder()
            .key("callback_data")
            .name("Callback Data")
            .required(false)
            .build()?)
        .display(ParameterDisplay::builder()
            .show_when("type", ParameterCondition::Eq(json!("callback_data")))
            .build())
        .build()?)
    .add_field("web_app", ObjectParameter::builder()
        .metadata(ParameterMetadata::builder()
            .key("web_app")
            .name("Web App")
            .required(false)
            .build()?)
        .add_field("url", TextParameter::builder()
            .metadata(ParameterMetadata::required("url", "Web App URL")?)
            .ui_options(TextUiOptions {
                input_type: TextInputType::URL,
                multiline: false,
            })
            .build()?)
        .display(ParameterDisplay::builder()
            .show_when("type", ParameterCondition::Eq(json!("web_app")))
            .build())
        .build()?)
    .layout(ObjectLayout::Vertical)
    .validate(|fields| {
        let button_type = fields.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let text = fields.get("text").and_then(|v| v.as_str()).unwrap_or("");
        
        if text.is_empty() {
            return Err("Button text is required".to_string());
        }
        
        if text.len() > 64 {
            return Err("Button text cannot exceed 64 characters".to_string());
        }
        
        match button_type {
            "url" => {
                let url = fields.get("url").and_then(|v| v.as_str()).unwrap_or("");
                if url.is_empty() {
                    return Err("URL is required for URL buttons".to_string());
                }
                if !url.starts_with("http://") && !url.starts_with("https://") {
                    return Err("URL must start with http:// or https://".to_string());
                }
            },
            "callback_data" => {
                let callback_data = fields.get("callback_data").and_then(|v| v.as_str()).unwrap_or("");
                if callback_data.is_empty() {
                    return Err("Callback data is required for callback buttons".to_string());
                }
                if callback_data.len() > 64 {
                    return Err("Callback data cannot exceed 64 characters".to_string());
                }
            },
            "web_app" => {
                let web_app = fields.get("web_app").and_then(|v| v.as_object());
                if let Some(web_app_obj) = web_app {
                    let url = web_app_obj.get("url").and_then(|v| v.as_str()).unwrap_or("");
                    if url.is_empty() {
                        return Err("Web App URL is required".to_string());
                    }
                    if !url.starts_with("https://") {
                        return Err("Web App URL must use HTTPS".to_string());
                    }
                } else {
                    return Err("Web App configuration is required".to_string());
                }
            },
            _ => {}
        }
        
        Ok(())
    })
    .build()?;
```

#### Key Benefits

**🎯 Semantic Cohesion:**
- All fields form a logical unit
- Cross-field validation and dependencies
- Clear data structure with meaning

**🔧 Type Safety:**
- Each field is strictly typed
- Validation at field and object levels
- Clear error messages for validation failures

**🎨 UI Flexibility:**
- Multiple layout options (Vertical, Horizontal, Grid)
- Field grouping for better organization
- Conditional field display based on other fields

**📊 Data Integrity:**
- Fixed structure prevents runtime errors
- Cross-field validation ensures data consistency
- Clear separation between structure and values

#### Design Patterns

**✅ Good Uses:**
```rust
// HTTP Header - always name + value
ObjectParameter::builder()
    .add_field("name", text_parameter!("name", "Name"))
    .add_field("value", text_parameter!("value", "Value"))

// Database Connection - related configuration
ObjectParameter::builder()
    .add_field("host", text_parameter!("host", "Host"))
    .add_field("port", number_parameter!("port", "Port"))
    .add_field("database", text_parameter!("database", "Database"))

// Coordinate - related position data
ObjectParameter::builder()
    .add_field("x", number_parameter!("x", "X"))
    .add_field("y", number_parameter!("y", "Y"))
    .add_field("z", number_parameter!("z", "Z"))
```

**❌ Anti-Patterns:**
```rust
// Don't use for unrelated fields
ObjectParameter::builder()
    .add_field("username", text_parameter!("username", "Username"))
    .add_field("color", color_parameter!("color", "Theme Color"))  // Unrelated!
    .add_field("timeout", number_parameter!("timeout", "Timeout"))  // Unrelated!

// Don't use for UI grouping only (use GroupParameter)
ObjectParameter::builder()
    .add_field("setting1", text_parameter!("setting1", "Setting 1"))
    .add_field("setting2", text_parameter!("setting2", "Setting 2"))
    // If these are just grouped for UI, use GroupParameter instead

// Don't use for different data types (use ModeParameter)
ObjectParameter::builder()
    .add_field("text_mode", text_parameter!("text", "Text"))
    .add_field("number_mode", number_parameter!("number", "Number"))
    // This should be a ModeParameter with different modes
```

#### Common Patterns

**HTTP Headers in Lists:**
```rust
let headers_list = ListParameter::new(
    ObjectParameter::builder()
        .metadata(ParameterMetadata::simple("header", "Header")?)
        .add_field("name", text_parameter!("name", "Name"))
        .add_field("value", text_parameter!("value", "Value"))
        .layout(ObjectLayout::Horizontal)
        .build()?
).build()?;
```

**Nested Objects:**
```rust
let api_config = ObjectParameter::builder()
    .add_field("endpoint", ObjectParameter::builder()
        .add_field("url", text_parameter!("url", "URL"))
        .add_field("method", select_parameter!("method", "Method", methods))
        .build()?)
    .add_field("auth", ObjectParameter::builder()
        .add_field("type", select_parameter!("type", "Auth Type", auth_types))
        .add_field("token", secret_parameter!("token", "Token"))
        .build()?)
    .build()?;
```

This approach ensures ObjectParameter is used for its intended purpose: representing cohesive data structures with meaningful relationships between fields.

---

### 22. ListParameter

**Purpose:** Dynamic arrays of independent parameter elements.

**When to use:**
- List of HTTP headers
- Multiple input values
- Telegram inline keyboard rows
- Database WHERE conditions
- Any collection of structured data

**Stored Data:** `ParameterValue::Array(Vec<ParameterValue>)` or `ParameterValue::Expression(String)`

**Expression Examples:**
- Static: `[{"name": "Accept", "value": "application/json"}, {"name": "User-Agent", "value": "MyApp"}]`
- Expression: `"{{$json.headers}}"` → dynamic header list from previous step

#### Architecture Principles

**🎯 Core Principle: Independent Elements**
- Each list item is completely independent
- No dependencies between elements
- Platform automatically generates technical IDs
- Clean separation of concerns

**🔧 Platform Responsibility:**
- Automatic ID generation for list items (`item_0`, `item_1`, etc.)
- UI management (add/remove/reorder)
- State management and persistence
- Animation and visual feedback

**👨‍💻 Developer Responsibility:**
- Define business structure through `item_template`
- Specify constraints and validation rules
- Focus on data types, not technical implementation

#### Core Structure

```rust
pub struct ListParameter {
    pub metadata: ParameterMetadata,
    
    /// Template for each list item - completely independent
    pub item_template: Parameter,
    
    /// Business constraints
    pub constraints: ListConstraints,
    
    /// UI configuration
    pub ui_config: ListUIConfig,
    
    /// Optional list-level validation
    pub validation: Option<ListValidation>,
}

pub struct ListConstraints {
    pub min_items: Option<usize>,
    pub max_items: Option<usize>,
    pub sortable: bool,
    pub unique_items: bool,
}

pub struct ListUIConfig {
    pub add_button_text: Option<String>,
    pub empty_text: Option<String>,
    pub show_indices: bool,
    pub show_delete: bool,
    pub show_reorder: bool,
    pub layout: ListLayout,
    pub animate: bool,
}

pub enum ListLayout {
    Vertical,
    Horizontal,
    Grid,
}
```

#### Builder API

```rust
impl ListParameter {
    /// Create simple list with template
    pub fn new(template: Parameter) -> ListParameterBuilder {
        ListParameterBuilder::new().item_template(template)
    }
    
    /// Create list builder
    pub fn builder() -> ListParameterBuilder {
        ListParameterBuilder::new()
    }
}

impl ListParameterBuilder {
    /// Set template for all items
    pub fn item_template(mut self, template: Parameter) -> Self {
        self.item_template = Some(template);
        self
    }
    
    /// Set business constraints
    pub fn constraints(mut self, constraints: ListConstraints) -> Self {
        self.constraints = constraints;
        self
    }
    
    /// Add list-level validation
    pub fn validate<F>(mut self, validate_fn: F) -> Self 
    where 
        F: Fn(&[ParameterValue]) -> Result<(), String> + 'static
    {
        self.validation = Some(ListValidation {
            validate_fn: Box::new(validate_fn),
        });
        self
    }
    
    /// Configure UI options
    pub fn ui_config(mut self, config: ListUIConfig) -> Self {
        self.ui_config = config;
        self
    }
    
    /// Build the parameter
    pub fn build(self) -> Result<ListParameter, ParameterError> {
        Ok(ListParameter {
            metadata: self.metadata.ok_or(ParameterError::MissingMetadata)?,
            item_template: self.item_template.ok_or(ParameterError::MissingTemplate)?,
            constraints: self.constraints,
            ui_config: self.ui_config,
            validation: self.validation,
        })
    }
}
```

#### Usage Examples

**Simple List of Strings:**
```rust
// Tags as text items
let tags = ListParameter::new(
    TextParameter::builder()
        .metadata(ParameterMetadata::required("tag", "Tag")?)
        .build()?
)
.metadata(ParameterMetadata::simple("tags", "Tags")?)
.constraints(ListConstraints {
    min_items: 1,
    max_items: Some(10),
    sortable: false,
    unique_items: true,
})
.build()?;
```

**Structured Objects:**
```rust
// HTTP headers
let headers = ListParameter::new(
    ObjectParameter::builder()
        .metadata(ParameterMetadata::simple("header", "Header")?)
        .add_field("name", TextParameter::builder()
            .metadata(ParameterMetadata::required("name", "Header Name")?)
            .build()?)
        .add_field("value", TextParameter::builder()
            .metadata(ParameterMetadata::required("value", "Header Value")?)
            .build()?)
        .build()?
)
.metadata(ParameterMetadata::simple("headers", "HTTP Headers")?)
.constraints(ListConstraints {
    min_items: 0,
    max_items: Some(50),
    sortable: true,
    unique_items: false,
})
.ui_config(ListUIConfig {
    add_button_text: Some("Add Header".into()),
    empty_text: Some("No headers configured".into()),
    show_indices: false,
    show_delete: true,
    show_reorder: true,
    layout: ListLayout::Vertical,
    animate: true,
})
.build()?;
```

**Nested Lists (Telegram Keyboard):**
```rust
// Telegram inline keyboard: List of rows, each row is a list of buttons
let inline_keyboard = ListParameter::new(
    // Each row is a list of buttons
    ListParameter::new(
        // Each button is an object
        ObjectParameter::builder()
            .metadata(ParameterMetadata::simple("button", "Button")?)
            .add_field("text", TextParameter::builder()
                .metadata(ParameterMetadata::required("text", "Button Text")?)
                .build()?)
            .add_field("type", SelectParameter::builder()
                .metadata(ParameterMetadata::required("type", "Button Type")?)
                .options(vec![
                    SelectOption::new("url", "URL"),
                    SelectOption::new("callback_data", "Callback Data"),
                ])
                .build()?)
            .add_field("url", TextParameter::builder()
                .metadata(ParameterMetadata::simple("url", "URL")?)
                .ui_options(TextUiOptions {
                    input_type: TextInputType::URL,
                    multiline: false,
                })
                .display(ParameterDisplay::builder()
                    .show_when("type", ParameterCondition::Eq(json!("url")))
                    .build())
                .build()?)
            .add_field("callback_data", TextParameter::builder()
                .metadata(ParameterMetadata::simple("callback_data", "Callback Data")?)
                .display(ParameterDisplay::builder()
                    .show_when("type", ParameterCondition::Eq(json!("callback_data")))
                    .build())
                .build()?)
            .build()?
    )
    .metadata(ParameterMetadata::simple("buttons", "Buttons in Row")?)
    .constraints(ListConstraints {
        min_items: 1,
        max_items: Some(5), // Telegram limit
        sortable: false,
        unique_items: false,
    })
    .build()?
)
.metadata(ParameterMetadata::simple("keyboard", "Inline Keyboard")?)
.constraints(ListConstraints {
    min_items: 0,
    max_items: Some(20), // Reasonable limit
    sortable: true,
    unique_items: false,
})
.ui_config(ListUIConfig {
    add_button_text: Some("Add Keyboard Row".into()),
    empty_text: Some("No keyboard rows".into()),
    show_indices: true,
    show_delete: true,
    show_reorder: true,
    layout: ListLayout::Vertical,
    animate: true,
})
.build()?;
```

**Complex List with Validation:**
```rust
// Database WHERE conditions with validation
let where_conditions = ListParameter::new(
    ObjectParameter::builder()
        .metadata(ParameterMetadata::simple("condition", "WHERE Condition")?)
        .add_field("field", TextParameter::builder()
            .metadata(ParameterMetadata::required("field", "Field Name")?)
            .build()?)
        .add_field("operator", SelectParameter::builder()
            .metadata(ParameterMetadata::required("operator", "Operator")?)
            .options(vec![
                SelectOption::new("=", "Equals"),
                SelectOption::new("!=", "Not Equals"),
                SelectOption::new(">", "Greater Than"),
                SelectOption::new("<", "Less Than"),
                SelectOption::new("LIKE", "Like"),
                SelectOption::new("IN", "In"),
            ])
            .build()?)
        .add_field("value", TextParameter::builder()
            .metadata(ParameterMetadata::required("value", "Value")?)
            .build()?)
        .build()?
)
.metadata(ParameterMetadata::simple("where", "WHERE Conditions")?)
.constraints(ListConstraints {
    min_items: 0,
    max_items: Some(10),
    sortable: true,
    unique_items: false,
})
.validate(|items| {
    // Validate no duplicate fields
    let mut fields = std::collections::HashSet::new();
    for item in items {
        if let Some(obj) = item.as_object() {
            if let Some(field) = obj.get("field").and_then(|v| v.as_str()) {
                if !fields.insert(field) {
                    return Err(format!("Duplicate field in WHERE conditions: {}", field));
                }
            }
        }
    }
    
    // Validate reasonable number of conditions
    if items.len() > 5 {
        return Err("Too many WHERE conditions. Consider using a more specific query.".to_string());
    }
    
    Ok(())
})
.build()?;
```

#### Key Benefits

**🎯 Clean Architecture:**
- No technical IDs in business logic
- Independent elements with clear boundaries
- Platform handles all technical concerns

**🔧 Developer-Friendly:**
- Simple template-based approach
- Focus on business structure, not implementation
- Powerful validation and constraints

**🎨 UI-Agnostic:**
- Platform handles all UI concerns
- Consistent behavior across all lists
- Automatic animations and state management

**📊 Data Flow:**
```
Developer defines template → Platform generates UI → User interacts → Platform manages state → Clean data to Action
```

#### Common Anti-Patterns to Avoid

**❌ Don't add technical IDs:**
```rust
// Wrong - don't add internal IDs
ObjectParameter::builder()
    .add_field("id", HiddenParameter::builder()...)  // Platform handles this
    .add_field("index", NumberParameter::builder()...)  // Platform handles this
```

**❌ Don't create dependencies between items:**
```rust
// Wrong - items should be independent
// Don't try to make item N depend on item N-1
```

**❌ Don't override platform UI concerns:**
```rust
// Wrong - let platform handle UI
.ui_config(ListUIConfig {
    custom_css: Some("..."),  // Platform handles styling
    custom_animations: Some("..."),  // Platform handles animations
})
```

**✅ Do focus on business logic:**
```rust
// Right - clean business structure
ListParameter::new(business_template)
    .constraints(business_constraints)
    .validate(business_validation)
```

This approach ensures clean separation of concerns and makes ListParameter truly universal for any collection of structured data.

---

### 23. RoutingParameter

**Purpose:** Wrapper that adds routing capabilities to list parameters, enabling dynamic connection points in node-based workflows.

**When to use:**
- Switch nodes with predefined cases
- Conditional routing based on static values
- Multi-output nodes where outputs are known at design time
- Any scenario where you need to generate visual connection points from a list of values

**When NOT to use:**
- Dynamic routing based on runtime data (use Action.outputs() instead)
- Database query outputs (use Action.outputs() instead)
- JSON parsing outputs (use Action.outputs() instead)
- Any scenario where outputs depend on input data analysis

**Stored Data:** `ParameterValue::Array(Vec<ParameterValue>)` (from internal ListParameter) or `ParameterValue::Expression(String)`

**Expression Examples:**
- Static: `["admin", "user", "guest"]` → generates admin, user, guest, default connection points
- Expression: `"{{$json.userRoles}}"` → dynamic case list from previous step

#### Architecture Principles

**🎯 Core Principle: Static Route Generation**
- Routes are generated from a predefined list of cases
- Each case in the list becomes a connection point in the node editor
- Connection points are known at design time, not runtime
- Automatic synchronization between list changes and visual outputs

**🔧 Visual Connection Points:**
- Adding a case to the list → new connection point appears
- Removing a case → connection point disappears
- Editing a case → connection point updates
- Platform automatically handles UI updates

**🎨 Design Time vs Runtime:**
- **Design Time:** RoutingParameter generates visual connection points
- **Runtime:** Action.execute() routes data to appropriate outputs
- **Dynamic Cases:** Use Action.outputs() for runtime-dependent routing

#### Core Structure

```rust
/// Parameter wrapper that adds routing capabilities to list parameters
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct RoutingParameter {
    /// Metadata for the routing parameter
    pub metadata: ParameterMetadata,
    
    /// List parameter containing the routing cases
    pub cases: ListParameter,
    
    /// Configuration for route generation
    #[builder(default)]
    pub routing_config: RoutingConfig,
    
    /// Current value of the cases list
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<ParameterValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// How to generate route names from list items
    pub route_naming: RouteNaming,
    
    /// Maximum number of routes to generate
    pub max_routes: Option<usize>,
    
    /// Whether to include default route
    pub include_default: bool,
    
    /// Default route configuration
    pub default_route: Option<DefaultRoute>,
    
    /// Route visual styling
    pub route_styling: RouteStyleConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultRoute {
    pub key: String,
    pub label: String,
    pub description: Option<String>,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RouteNaming {
    /// Use list item values directly as route names
    UseItemValues,
    
    /// Use specific field from object items
    UseItemField { field_name: String },
    
    /// Use template with placeholders
    Template { template: String },
    
    /// Sequential naming with prefix
    Sequential { prefix: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteStyleConfig {
    /// Default color for route connection points
    pub default_color: String,
    
    /// Color for default route
    pub default_route_color: String,
    
    /// Connection line thickness
    pub line_thickness: u32,
    
    /// Connection line pattern
    pub line_pattern: LinePattern,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LinePattern {
    Solid,
    Dashed,
    Dotted,
}

/// Visual connection point in node editor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionPoint {
    /// Unique identifier for the connection point
    pub id: String,
    
    /// Display label
    pub label: String,
    
    /// Optional description
    pub description: Option<String>,
    
    /// Position on the node
    pub position: ConnectionPosition,
    
    /// Data type flowing through this connection
    pub data_type: DataType,
    
    /// Optional icon
    pub icon: Option<String>,
    
    /// Visual styling
    pub style: ConnectionStyle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConnectionPosition {
    Right(usize),  // Right side of node (outputs)
    Left(usize),   // Left side of node (inputs)
    Top(usize),    // Top of node
    Bottom(usize), // Bottom of node
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionStyle {
    pub color: String,
    pub thickness: u32,
    pub pattern: LinePattern,
}
```

#### Builder API

```rust
impl RoutingParameter {
    /// Create routing parameter with simple text cases
    pub fn with_text_cases(metadata: ParameterMetadata) -> RoutingParameterBuilder {
        RoutingParameterBuilder::new()
            .metadata(metadata)
            .cases(
                ListParameter::new(
                    TextParameter::builder()
                        .metadata(ParameterMetadata::required("case", "Case Value")?)
                        .build()?
                )
                .build()?
            )
    }
    
    /// Create routing parameter with object cases
    pub fn with_object_cases(metadata: ParameterMetadata, object_template: ObjectParameter) -> RoutingParameterBuilder {
        RoutingParameterBuilder::new()
            .metadata(metadata)
            .cases(
                ListParameter::new(Parameter::Object(object_template))
                    .build()?
            )
    }
    
    /// Create builder
    pub fn builder() -> RoutingParameterBuilder {
        RoutingParameterBuilder::new()
    }
}

impl RoutingParameterBuilder {
    /// Set the cases list parameter
    pub fn cases(mut self, cases: ListParameter) -> Self {
        self.cases = Some(cases);
        self
    }
    
    /// Set routing configuration
    pub fn routing_config(mut self, config: RoutingConfig) -> Self {
        self.routing_config = config;
        self
    }
    
    /// Enable default route
    pub fn include_default(mut self, default_route: DefaultRoute) -> Self {
        self.routing_config.include_default = true;
        self.routing_config.default_route = Some(default_route);
        self
    }
    
    /// Set route naming strategy
    pub fn route_naming(mut self, naming: RouteNaming) -> Self {
        self.routing_config.route_naming = naming;
        self
    }
    
    /// Set maximum routes
    pub fn max_routes(mut self, max: usize) -> Self {
        self.routing_config.max_routes = Some(max);
        self
    }
    
    /// Build the parameter
    pub fn build(self) -> Result<RoutingParameter, ParameterError> {
        Ok(RoutingParameter {
            metadata: self.metadata.ok_or(ParameterError::MissingMetadata)?,
            cases: self.cases.ok_or(ParameterError::MissingCases)?,
            routing_config: self.routing_config,
            value: self.value,
        })
    }
}
```

#### Connection Point Generation

```rust
impl RoutingParameter {
    /// Generate connection points from current cases
    pub fn generate_connection_points(&self) -> Result<Vec<ConnectionPoint>, RouteError> {
        let cases_value = self.value.as_ref()
            .ok_or_else(|| RouteError::NoValue)?;
        
        let cases_array = cases_value.as_array()
            .ok_or_else(|| RouteError::InvalidCasesFormat)?;
        
        let mut connection_points = Vec::new();
        
        // Generate connection points from cases
        for (index, case_item) in cases_array.iter().enumerate() {
            let route_name = self.extract_route_name(case_item)?;
            
            connection_points.push(ConnectionPoint {
                id: route_name.clone(),
                label: route_name.clone(),
                description: Some(format!("Route for case: {}", route_name)),
                position: ConnectionPosition::Right(index),
                data_type: DataType::Any,
                icon: Some("arrow-right".to_string()),
                style: ConnectionStyle {
                    color: self.routing_config.route_styling.default_color.clone(),
                    thickness: self.routing_config.route_styling.line_thickness,
                    pattern: self.routing_config.route_styling.line_pattern.clone(),
                },
            });
        }
        
        // Add default route if configured
        if self.routing_config.include_default {
            if let Some(default_route) = &self.routing_config.default_route {
                connection_points.push(ConnectionPoint {
                    id: default_route.key.clone(),
                    label: default_route.label.clone(),
                    description: default_route.description.clone(),
                    position: ConnectionPosition::Right(connection_points.len()),
                    data_type: DataType::Any,
                    icon: default_route.icon.clone(),
                    style: ConnectionStyle {
                        color: self.routing_config.route_styling.default_route_color.clone(),
                        thickness: self.routing_config.route_styling.line_thickness,
                        pattern: LinePattern::Dashed,
                    },
                });
            }
        }
        
        // Apply max routes limit
        if let Some(max_routes) = self.routing_config.max_routes {
            connection_points.truncate(max_routes);
        }
        
        Ok(connection_points)
    }
    
    /// Extract route name from case item based on naming strategy
    fn extract_route_name(&self, case_item: &ParameterValue) -> Result<String, RouteError> {
        match &self.routing_config.route_naming {
            RouteNaming::UseItemValues => {
                case_item.as_str()
                    .ok_or_else(|| RouteError::InvalidItemFormat)
                    .map(|s| s.to_string())
            },
            RouteNaming::UseItemField { field_name } => {
                case_item.as_object()
                    .and_then(|obj| obj.get(field_name))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RouteError::MissingRouteField(field_name.clone()))
                    .map(|s| s.to_string())
            },
            RouteNaming::Template { template } => {
                // Simple template substitution
                let item_str = case_item.as_str().unwrap_or("unknown");
                Ok(template.replace("{value}", item_str))
            },
            RouteNaming::Sequential { prefix } => {
                let item_str = case_item.as_str().unwrap_or("unknown");
                Ok(format!("{}_{}", prefix, item_str))
            },
        }
    }
    
    /// Subscribe to changes in cases list
    pub fn subscribe_to_changes<F>(&self, callback: F) 
    where 
        F: Fn(Vec<ConnectionPoint>) + 'static
    {
        // When cases list changes, regenerate connection points
        self.cases.on_change(move |_| {
            if let Ok(new_connection_points) = self.generate_connection_points() {
                callback(new_connection_points);
            }
        });
    }
}
```

#### Usage Examples

**Simple Switch Node:**
```rust
let switch_cases = RoutingParameter::with_text_cases(
    ParameterMetadata::required("switch_cases", "Switch Cases")?
)
.routing_config(RoutingConfig {
    route_naming: RouteNaming::UseItemValues,
    max_routes: Some(20),
    include_default: true,
    default_route: Some(DefaultRoute {
        key: "default".to_string(),
        label: "Default".to_string(),
        description: Some("When no cases match".to_string()),
        icon: Some("arrow-down".to_string()),
    }),
    route_styling: RouteStyleConfig {
        default_color: "#4CAF50".to_string(),
        default_route_color: "#FF9800".to_string(),
        line_thickness: 2,
        line_pattern: LinePattern::Solid,
    },
})
.build()?;
```

**Complex Object-Based Routing:**
```rust
let object_routing = RoutingParameter::with_object_cases(
    ParameterMetadata::required("complex_routes", "Complex Routes")?,
    ObjectParameter::builder()
        .metadata(ParameterMetadata::simple("route_config", "Route Config")?)
        .add_field("route_name", TextParameter::builder()
            .metadata(ParameterMetadata::required("route_name", "Route Name")?)
            .build()?)
        .add_field("condition", TextParameter::builder()
            .metadata(ParameterMetadata::required("condition", "Condition")?)
            .build()?)
        .add_field("priority", NumberParameter::builder()
            .metadata(ParameterMetadata::optional("priority", "Priority")?)
            .default(1.0)
            .build()?)
        .build()?
)
.routing_config(RoutingConfig {
    route_naming: RouteNaming::UseItemField { 
        field_name: "route_name".to_string() 
    },
    max_routes: Some(50),
    include_default: true,
    default_route: Some(DefaultRoute {
        key: "fallback".to_string(),
        label: "Fallback".to_string(),
        description: Some("When no conditions match".to_string()),
        icon: Some("shield".to_string()),
    }),
    route_styling: RouteStyleConfig {
        default_color: "#2196F3".to_string(),
        default_route_color: "#F44336".to_string(),
        line_thickness: 3,
        line_pattern: LinePattern::Solid,
    },
})
.build()?;
```

**Template-Based Route Naming:**
```rust
let templated_routing = RoutingParameter::with_text_cases(
    ParameterMetadata::required("templated_routes", "Templated Routes")?
)
.routing_config(RoutingConfig {
    route_naming: RouteNaming::Template { 
        template: "output_{value}".to_string() 
    },
    max_routes: Some(10),
    include_default: false,
    default_route: None,
    route_styling: RouteStyleConfig {
        default_color: "#9C27B0".to_string(),
        default_route_color: "#607D8B".to_string(),
        line_thickness: 2,
        line_pattern: LinePattern::Dotted,
    },
})
.build()?;
```

#### Integration with Actions

**Action Definition:**
```rust
pub fn create_switch_node() -> ActionDefinition {
    ActionDefinition::builder()
        .name("Switch")
        .description("Route data based on switch cases")
        .parameters(vec![
            // Input value for comparison
            Parameter::Text(
                TextParameter::builder()
                    .metadata(ParameterMetadata::required("input_value", "Input Value")?)
                    .build()?
            ),
            
            // Routing parameter with cases
            Parameter::Routing(
                RoutingParameter::with_text_cases(
                    ParameterMetadata::required("cases", "Switch Cases")?
                )
                .include_default(DefaultRoute {
                    key: "default".to_string(),
                    label: "Default".to_string(),
                    description: Some("When input doesn't match any case".to_string()),
                    icon: Some("arrow-down".to_string()),
                })
                .max_routes(20)
                .build()?
            ),
        ])
        .build()
}
```

**Action Implementation:**
```rust
impl Action for SwitchAction {
    fn execute(&self, params: &ParameterValues) -> Result<ExecutionResult, Error> {
        let input_value = params.get("input_value")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::MissingParameter("input_value".into()))?;
        
        let cases = params.get("cases")
            .and_then(|v| v.as_array())
            .ok_or_else(|| Error::MissingParameter("cases".into()))?;
        
        let input_data = params.get("input_data");
        
        // Check each case for match
        for case_value in cases {
            if let Some(case_str) = case_value.as_str() {
                if input_value == case_str {
                    // Found matching case - route to corresponding output
                    return Ok(ExecutionResult {
                        outputs: vec![
                            (case_str.to_string(), input_data.cloned())
                        ].into_iter().collect(),
                        status: ExecutionStatus::Success,
                    });
                }
            }
        }
        
        // No matching case - route to default output
        Ok(ExecutionResult {
            outputs: vec![
                ("default".to_string(), input_data.cloned())
            ].into_iter().collect(),
            status: ExecutionStatus::Success,
        })
    }
}
```

#### Visual Representation

**Node Editor Display:**
```
┌─────────────────────────────────────────┐
│              Switch Node                │
├─────────────────────────────────────────┤
│ Input Value: [user_type_______________] │
│                                         │
│ Switch Cases:                           │
│   ┌─────────────────────────────────┐   │
│   │ admin                    [Del]  │   │  ●─── admin
│   │ user                     [Del]  │   │  ●─── user
│   │ guest                    [Del]  │   │  ●─── guest
│   │ moderator                [Del]  │   │  ●─── moderator
│   └─────────────────────────────────┘   │
│   [Add Case]                            │  ●─── default
│                                         │
└─────────────────────────────────────────┘
```

**Real-time Updates:**
- Add case "admin" → new connection point appears on right
- Remove case "user" → connection point disappears
- Edit case "guest" → connection point label updates
- All changes are synchronized automatically

#### Key Benefits

**🎯 Static Design-Time Routing:**
- Connection points are known at design time
- Visual feedback for all possible routes
- No runtime surprises about available outputs

**🔄 Automatic Synchronization:**
- List changes instantly update connection points
- No manual refresh or rebuild required
- Consistent UI state management

**🎨 Visual Clarity:**
- Clear connection between parameter configuration and node outputs
- Intuitive relationship between cases and routes
- Immediate visual feedback for configuration changes

**🛠️ Developer-Friendly:**
- Simple API for common routing scenarios
- Flexible configuration for complex cases
- Clean separation from runtime routing logic

#### Common Patterns

**User Role Routing:**
```rust
let user_roles = RoutingParameter::with_text_cases(
    ParameterMetadata::required("user_roles", "User Roles")?
)
.include_default(DefaultRoute {
    key: "anonymous".to_string(),
    label: "Anonymous".to_string(),
    description: Some("Users without specific roles".to_string()),
    icon: Some("user".to_string()),
})
.build()?;
```

**HTTP Status Code Routing:**
```rust
let http_status = RoutingParameter::with_text_cases(
    ParameterMetadata::required("status_codes", "HTTP Status Codes")?
)
.routing_config(RoutingConfig {
    route_naming: RouteNaming::Template { 
        template: "status_{value}".to_string() 
    },
    include_default: true,
    default_route: Some(DefaultRoute {
        key: "unexpected".to_string(),
        label: "Unexpected".to_string(),
        description: Some("Unexpected status codes".to_string()),
        icon: Some("warning".to_string()),
    }),
    // ... other config
})
.build()?;
```

#### Design Principles

**✅ Good Uses:**
- Switch/case logic with predefined values
- Multi-output nodes with known outputs
- Conditional routing based on static configuration
- Any scenario where routes can be determined at design time

**❌ Avoid For:**
- Dynamic routing based on input data analysis
- Database query result routing
- JSON object key routing
- Any scenario where routes depend on runtime data

For dynamic routing scenarios, use `Action.outputs()` method instead, which has access to runtime data and can generate outputs dynamically.

This approach ensures clean separation between design-time configuration (RoutingParameter) and runtime behavior (Action.outputs()).

**Purpose:** Switch between different input modes.

**When to use:**
- Simple selection vs custom input
- Different complexity levels
- Adaptive interfaces

**Stored Data:** Value depends on selected mode - can be any `ParameterValue` type or `ParameterValue::Expression(String)`

**Note:** ModeParameter stores the value from whichever mode is currently active (text, select, code, etc.)

```rust
// Flexible URL input
let url_input = ModeParameter::builder()
    .metadata(metadata)
    .text_mode("Custom", custom_text_param)
    .select_mode("Predefined", predefined_urls)
    .expression_mode("Dynamic", expression_param)
    .default_mode(ModeType::Select)
    .ui_options(ModeUiOptions::tabs())
    .build()?;
```

## 🔄 Special Parameters

### 25. ExpirableParameter<T>

**Purpose:** Wrapper for parameters with time-to-live.

**When to use:**
- OAuth tokens
- Temporary keys
- Cached data

**Stored Data:** Wrapped parameter's `ParameterValue` plus expiration metadata

**Note:** The wrapped parameter can use expressions, and the expiration logic is handled separately.

```rust
// Expiring token
let token = ExpirableParameter::new(
    SecretParameter::builder()
        .metadata(metadata)
        .build()?,
    Duration::hours(1)
);
```

## 🔧 Removed Types and Alternatives

### Removed: TextareaParameter

**Why removed:** Merged into `TextParameter` with `multiline: true` option.

**Migration:** Use `TextParameter` with multiline enabled:
```rust
// Old TextareaParameter
let description = TextareaParameter::builder()...

// New approach
let description = TextParameter::builder()
    .metadata(metadata)
    .ui_options(TextUiOptions {
        multiline: true,
        rows: Some(5), // Only when critical
    })
    .build()?;
```

### Removed: ExpressionParameter

**Why removed:** Universal expression support makes this redundant.

**Migration:** Use appropriate typed parameters with expression toggle provided by platform core.

### Alternative Approaches for Common Needs

#### Cron Expressions

**Instead of special CronParameter, use TextParameter with validation:**

```rust
// Cron schedule via regular text
let schedule = TextParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("schedule")
        .name("Schedule (Cron Expression)")
        .description("Cron expression for scheduling tasks")
        .placeholder("0 9 * * 1-5")
        .hint("Format: minute hour day month weekday (e.g., '0 9 * * *' for daily at 9 AM)")
        .build()?)
    .validation(ParameterValidation::builder()
        .pattern(r"^(\*|[0-5]?\d) (\*|[01]?\d|2[0-3]) (\*|[0-2]?\d|3[01]) (\*|[0]?\d|1[0-2]) (\*|[0-6])$")
        .custom_validator(validate_cron_expression)
        .build())
    .build()?;

// Cron expression validation
fn validate_cron_expression(value: &str) -> Result<(), ValidationError> {
    match cron::Schedule::from_str(value) {
        Ok(_) => Ok(()),
        Err(e) => Err(ValidationError::Custom(
            format!("Invalid cron expression: {}", e)
        )),
    }
}
```

**Advantages:**
- Simple architecture without extra types
- Flexibility for advanced users
- Standard validation
- UI help can be added in platform

#### Tags and Keywords

**Instead of TagsParameter, use existing types:**

```rust
// Option 1: MultiSelect with ability to create new
let tags = MultiSelectParameter::builder()
    .metadata(ParameterMetadata::optional("tags", "Article Tags")?)
    .options(predefined_tags)
    .ui_options(MultiSelectUiOptions {
        creatable: true,  // Can create new tags
        searchable: true,
    })
    .build()?;

// Option 2: Simple text with separators
let keywords = TextParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("keywords")
        .name("Keywords")
        .description("Enter keywords separated by commas")
        .placeholder("keyword1, keyword2, keyword3")
        .hint("Separate multiple keywords with commas")
        .build()?)
    .validation(ParameterValidation::builder()
        .custom_validator(validate_comma_separated_tags)
        .build())
    .build()?;

// Option 3: List of simple text fields
let tag_list = ListParameter::new(
    TextParameter::builder()
        .metadata(ParameterMetadata::simple("tag", "Tag")?)
        .build()?
)
.metadata(ParameterMetadata::simple("tags", "Tags")?)
.constraints(ListConstraints {
    min_items: 0,
    max_items: Some(10),
    sortable: false,
    unique_items: true,
})
.build()?;
```

#### JSON Data

**For structured data, use ObjectParameter + ListParameter combinations:**
```rust
// Instead of free-form JSON, build structure with typed parameters
let webhook_config = ObjectParameter::builder()
    .add_field("event", select_parameter!("event", "Event Type", events))
    .add_field("data", object_parameter!("data", user_data_fields))
    .add_field("metadata", list_parameter!("metadata", metadata_items))
    .build()?;
```

**For free-form JSON, use CodeParameter with JSON language:**
```rust
// When free-form JSON is actually needed
let custom_payload = CodeParameter::builder()
    .metadata(ParameterMetadata::simple("payload", "Custom JSON")?)
    .ui_options(CodeUiOptions {
        language: CodeLanguage::JSON,
    })
    .build()?;
```

#### Webhook URLs

**For outgoing webhooks, use regular TextParameter:**
```rust
let webhook_url = TextParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("webhook_url")
        .name("Webhook URL")
        .description("URL to send webhook notifications")
        .placeholder("https://your-service.com/webhook")
        .build()?)
    .ui_options(TextUiOptions {
        input_type: TextInputType::URL,
        multiline: false,
    })
    .validation(ParameterValidation::builder()
        .url_validation()
        .build())
    .build()?;
```

**For incoming webhooks, implement logic in WebhookAction:**
```rust
pub struct GitHubWebhookAction {
    // Platform generates URL automatically
    // Developer implements subscribe/unsubscribe
}

impl WebhookAction for GitHubWebhookAction {
    async fn subscribe(&self, webhook_url: String) -> Result<(), Error> {
        // GitHub webhook subscription logic
    }
    
    async fn unsubscribe(&self, webhook_url: String) -> Result<(), Error> {
        // GitHub webhook unsubscription logic  
    }
}
```

## 📚 Real Action Examples

**Webhook logic is implemented in WebhookAction, not in parameters:**

```rust
// For outgoing webhooks use regular TextParameter
let webhook_url = TextParameter::builder()
    .metadata(ParameterMetadata::builder()
        .key("webhook_url")
        .name("Webhook URL")
        .description("URL to send webhook notifications")
        .placeholder("https://your-service.com/webhook")
        .build()?)
    .ui_options(TextUiOptions {
        input_type: TextInputType::URL,
        multiline: false,
        rows: None,
    })
    .validation(ParameterValidation::builder()
        .url_validation()
        .build())
    .build()?;

// For incoming webhooks logic in WebhookAction:
pub struct GitHubWebhookAction {
    // Platform generates URL automatically
    // Developer implements subscribe/unsubscribe
}

impl WebhookAction for GitHubWebhookAction {
    async fn subscribe(&self, webhook_url: String) -> Result<(), Error> {
        // GitHub webhook subscription logic
    }
    
    async fn unsubscribe(&self, webhook_url: String) -> Result<(), Error> {
        // GitHub webhook unsubscription logic  
    }
}
```

### HTTP Request Action

```rust
pub fn create_http_request_action() -> ActionDefinition {
    ActionDefinition::builder()
        .name("HTTP Request")
        .description("Make HTTP requests to external APIs")
        .parameters(vec![
            // URL
            Parameter::Text(
                TextParameter::builder()
                    .metadata(ParameterMetadata::builder()
                        .key("url")
                        .name("URL")
                        .required(true)
                        .description("The URL to make the request to")
                        .placeholder("https://api.example.com/users")
                        .build()?)
                    .ui_options(TextUiOptions {
                        input_type: TextInputType::URL,
                        multiline: false,
                        rows: None,
                    })
                    .build()?
            ),
            
            // HTTP Method
            Parameter::Radio(
                RadioParameter::builder()
                    .metadata(ParameterMetadata::builder()
                        .key("method")
                        .name("HTTP Method")
                        .required(true)
                        .build()?)
                    .options(vec![
                        RadioOption::new("GET", "GET", "Retrieve data"),
                        RadioOption::new("POST", "POST", "Send data"),
                        RadioOption::new("PUT", "PUT", "Update data"),
                        RadioOption::new("DELETE", "DELETE", "Delete data"),
                    ])
                    .ui_options(RadioUiOptions {
                        layout: RadioLayout::Horizontal,
                        show_descriptions: true,
                        show_icons: false,
                    })
                    .build()?
            ),
            
            // Headers
            Parameter::List(
                ParameterList::builder()
                    .metadata(ListMetadata::new("headers", "HTTP Headers"))
                    .item_template(
                        Parameter::Object(
                            ObjectParameter::builder()
                                .metadata(ObjectMetadata::new("header", "Header"))
                                .add_field("name", text_parameter!("name", "Header Name"))?
                                .add_field("value", text_parameter!("value", "Header Value"))?
                                .build()?
                        )
                    )
                    .constraints(ListConstraints {
                        min_items: 0,
                        max_items: Some(50),
                        sortable: true,
                        unique_items: false,
                    })
                    .build()?
            ),
            
            // Request Body
            Parameter::Code(
                CodeParameter::builder()
                    .metadata(ParameterMetadata::builder()
                        .key("body")
                        .name("Request Body")
                        .required(false)
                        .description("JSON body for POST/PUT requests")
                        .build()?)
                    .ui_options(CodeUiOptions {
                        language: CodeLanguage::JSON,
                        height: 8,
                        available_variables: vec![
                            "$json".into(),
                            "$input".into(),
                        ],
                    })
                    .display(ParameterDisplay::builder()
                        .show_when("method", ParameterCondition::Or(vec![
                            ParameterCondition::Eq(json!("POST")),
                            ParameterCondition::Eq(json!("PUT")),
                            ParameterCondition::Eq(json!("PATCH")),
                        ]))
                        .build())
                    .build()?
            ),
            
            // Timeout
            Parameter::Number(
                NumberParameter::builder()
                    .metadata(ParameterMetadata::builder()
                        .key("timeout")
                        .name("Timeout")
                        .required(false)
                        .description("Request timeout in seconds")
                        .build()?)
                    .default(30.0)
                    .ui_options(NumberUiOptions {
                        format: NumberFormat::Integer,
                        min: Some(1.0),
                        max: Some(300.0),
                        step: Some(1.0),
                        unit: Some("seconds".into()),
                    })
                    .build()?
            ),
        ])
        .build()
}
```

### Slack Send Message Action

```rust
pub fn create_slack_message_action() -> ActionDefinition {
    ActionDefinition::builder()
        .name("Send Slack Message")
        .description("Send a message to a Slack channel")
        .credential_type("slack_oauth2")
        .parameters(vec![
            // Channel
            Parameter::Resource(
                ResourceParameter::dependent_resource()
                    .metadata(ParameterMetadata::required("channel", "Channel")?)
                    .depends_on(vec!["credential"])
                    .load_with(|ctx| Box::pin(async move {
                        let credential = ctx.credentials.get("slack_oauth2")
                            .ok_or_else(|| LoadError::MissingCredential("slack_oauth2".into()))?;
                        
                        let response = ctx.http_client
                            .get("https://slack.com/api/conversations.list")
                            .header("Authorization", format!("Bearer {}", credential.token))
                            .query(&[("types", "public_channel,private_channel")])
                            .send()
                            .await?;
                            
                        let data: serde_json::Value = response.json().await?;
                        let channels = data["channels"].as_array()
                            .ok_or_else(|| LoadError::InvalidResponse("Missing channels array".into()))?;
                        
                        let mut items = Vec::new();
                        for channel in channels {
                            if let Some(id) = channel["id"].as_str() {
                                let name = channel["name"].as_str().unwrap_or("Unknown");
                                let is_private = channel["is_private"].as_bool().unwrap_or(false);
                                
                                items.push(ResourceItem {
                                    id: id.to_string(),
                                    label: format!("#{}", name),
                                    description: channel["purpose"]["value"].as_str().map(String::from),
                                    icon: Some(ResourceIcon::Icon(
                                        if is_private { "lock" } else { "hash" }.to_string()
                                    )),
                                    metadata: {
                                        let mut map = serde_json::Map::new();
                                        map.insert("is_private".into(), json!(is_private));
                                        map.insert("name".into(), json!(name));
                                        map
                                    },
                                    enabled: true,
                                    group: Some(if is_private { "Private" } else { "Public" }.to_string()),
                                    sort_key: Some(name.to_lowercase()),
                                });
                            }
                        }
                        
                        Ok(items)
                    }))
                    .cache(Duration::minutes(5))
                    .loading_strategy(LoadingStrategy::OnDemand)
                    .build()?
            ),
            
            // Message text
            Parameter::Mode(
                ModeParameter::builder()
                    .metadata(ParameterMetadata::required("text", "Message")?)
                    .text_mode("Simple", simple_text_param)
                    .expression_mode("Dynamic", expression_param)
                    .code_mode("Rich", html_param)
                    .default_mode(ModeType::Text)
                    .ui_options(ModeUiOptions::tabs())
                    .build()?
            ),
            
            // Schedule
            Parameter::DateTime(
                DateTimeParameter::builder()
                    .metadata(ParameterMetadata::builder()
                        .key("schedule_time")
                        .name("Schedule Time")
                        .required(false)
                        .description("When to send the message")
                        .build()?)
                    .ui_options(DateTimeUiOptions {
                        mode: DateTimeMode::DateTime,
                        timezone: TimezoneHandling::UserLocal,
                        min_date: Some(today()),
                        max_date: None,
                    })
                    .build()?
            ),
            
            // Message color
            Parameter::Color(
                ColorParameter::builder()
                    .metadata(ParameterMetadata::builder()
                        .key("color")
                        .name("Message Color")
                        .required(false)
                        .build()?)
                    .ui_options(ColorUiOptions {
                        format: ColorFormat::Hex,
                        palette: vec![
                            "#36a64f".into(), // good
                            "#ff0000".into(), // danger
                            "#ffaa00".into(), // warning
                            "#439fe0".into(), // info
                        ],
                        alpha: false,
                    })
                    .build()?
            ),
        ])
        .build()
}
```

## 🎯 Usage Recommendations

### Choosing the Right Parameter Type

1. **For simple text** → `TextParameter`
2. **For multi-line text** → `TextareaParameter`  
3. **For secrets** → `SecretParameter` 
4. **For numbers** → `NumberParameter`
5. **For boolean values** → `BooleanParameter` or `CheckboxParameter`
6. **For list selection** → `SelectParameter`
7. **For multiple selection** → `MultiSelectParameter`
8. **For exclusive choice** → `RadioParameter`
9. **For date and time** → `DateTimeParameter`
10. **For date only** → `DateParameter`
11. **For time only** → `TimeParameter`
12. **For code** → `CodeParameter`
13. **For expressions** → `ExpressionParameter`
14. **For external resources** → `ResourceParameter`
15. **For JSON** → `JSONParameter`
16. **For files** → `FileParameter`
15. **For colors** → `ColorParameter`
16. **For hidden data** → `HiddenParameter`
17. **For messages** → `NoticeParameter`
18. **For grouping** → `GroupParameter`
19. **For arrays** → `ListParameter`
20. **For objects** → `ObjectParameter`
21. **For mode switching** → `ModeParameter`
22. **For routing/connection points** → `RoutingParameter`
23. **For TTL data** → `ExpirableParameter<T>`
24. **For cron schedules** → `TextParameter` with cron validation
25. **For tags** → `MultiSelectParameter` with creatable or `TextParameter` with separators

### Design Principles

- **Minimal UI Options** - only business-critical settings that fundamentally change behavior
- **Platform Core Responsibility** - core automatically handles all standard functionality
- **Clean Architecture** - parameters focus on data types and business logic, not visual appearance
- **Clear Naming** - understandable keys and names
- **Proper Validation** - business rules and constraints at parameter level
- **Logical Grouping** - related parameters together

### 🎯 What Parameters Should Define

**Business Logic & Data:**
- Data types and validation rules
- Required fields and constraints (min/max values, lengths)
- Critical behavioral differences (single-line vs multi-line)
- Domain-specific options (language for syntax highlighting)
- Business rules (creatable options, file type restrictions)

### 🚫 What Parameters Should NOT Define

**Visual & Standard Behavior (Platform Handles):**
- Heights, widths, and sizing (`height: 6`, `cols: 80`, `rows: 10`)
- Colors, themes, and visual styling
- Expression functionality (`available_variables`, `show_preview`)
- Auto-completion and hint systems
- Character counters and format helpers
- Loading states and animations
- Error styling and feedback presentation
- Responsive behavior and accessibility

### ⚡ Platform Core Auto-Features

**Expression Support:**
- Automatic toggle buttons between static/expression modes
- Variable auto-completion ($json, $node, $workflow, etc.)
- Expression syntax highlighting and validation
- Preview of expression results

**Standard Behaviors:**
- Optimal sizing based on content and screen size
- Consistent styling across all parameters
- Loading states for async operations (ResourceParameter)
- Error handling and user feedback
- Accessibility compliance (ARIA labels, keyboard navigation)
- Mobile responsiveness

**Development Guidelines:**
- If you're tempted to add a UI option, ask: "Does this change business logic or just appearance?"
- If it's appearance/behavior that should be consistent across the platform → Platform handles it
- If it's a business rule or fundamentally changes how data is processed → Parameter should define it
- When in doubt, omit the option - the platform provides sensible defaults

This keeps the parameter system focused on what matters: defining clean, type-safe data structures for your workflows.

## 🚀 Conclusion

The Nebula parameter system provides powerful tools for creating type-safe and user-friendly configuration forms. Each parameter type solves specific tasks, while universal expression support through the platform core enables dynamic workflows without complicating the architecture.

### 🎯 Key Benefits

- **Clean Architecture:** Parameters focus purely on data types and business logic
- **Platform Responsibility:** Core handles all visual and standard behaviors automatically
- **Universal Expression Support:** Any parameter can be static or dynamic without extra configuration
- **Type Safety:** Strong typing with automatic validation and transformation
- **User-Friendly:** Two-field approach prevents data loss during mode switching
- **Execution Pipeline:** Clear Transform → Validation → Process flow
- **Developer-Friendly:** Minimal API surface with sensible defaults

### 📊 Expression Data Flow

**Database Storage:** Clean `ParameterValue` types including `Expression(String)`
**Client Interface:** Two-field approach for optimal UX (static_value + expression_value)
**Platform Detection:** Automatic mode detection and expression validation
**Execution:** Seamless Transform → Validation → Process pipeline

### 🛡️ Architecture Guardrails

**Remember:** This system is designed to be simple and focused. Resist the urge to add UI options that the platform should handle automatically. When you see parameters with many UI options in other systems, ask yourself:

- "Is this a business rule or visual preference?"
- "Should this be consistent across the platform?"
- "Does this change how data is processed or just how it looks?"

**Keep parameters clean, focused, and minimal.** The platform core is your partner in creating excellent user experiences.

### 📚 Usage Reference

Use this documentation as a reference when creating Actions and Credentials for your workflow platform. Focus on defining clear data requirements and business rules - let the platform handle the rest.

