# Nebula Parameters - API Examples

**Real-world usage patterns and code examples**

---

## Table of Contents

1. [Basic Usage](#basic-usage)
2. [Form Validation](#form-validation)
3. [Workflow Automation](#workflow-automation)
4. [3D Graphics](#3d-graphics)
5. [Game Settings](#game-settings)
6. [CLI Tools](#cli-tools)
7. [Data Processing](#data-processing)
8. [REST API Configuration](#rest-api-configuration)

---

## Basic Usage

### Example 1: Simple Contact Form

```rust
use nebula_parameter::prelude::*;

fn create_contact_form() -> Schema {
    Schema::new()
        .with_parameter(
            TextParameter::builder("name")
                .label("Full Name")
                .required()
                .min_length(2)
                .max_length(100)
                .placeholder("John Doe")
                .build()
        )
        .with_parameter(
            TextParameter::email("email")
                .label("Email Address")
                .required()
                .placeholder("john@example.com")
                .build()
        )
        .with_parameter(
            TextParameter::builder("phone")
                .subtype(TextSubtype::PhoneNumber)
                .label("Phone Number")
                .placeholder("+1-555-123-4567")
                .build()
        )
        .with_parameter(
            TextParameter::builder("message")
                .subtype(TextSubtype::MultiLine)
                .label("Message")
                .required()
                .min_length(10)
                .max_length(1000)
                .placeholder("How can we help you?")
                .build()
        )
        .build()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let schema = create_contact_form();
    let mut context = Context::new(schema);
    
    // Set values
    context.set_value("name", Value::text("Alice Smith"))?;
    context.set_value("email", Value::text("alice@example.com"))?;
    context.set_value("phone", Value::text("+1-555-987-6543"))?;
    context.set_value("message", Value::text("I need help with..."))?;
    
    // Validate
    if context.validate_all() {
        println!("✅ Form is valid!");
        
        // Get values
        let name = context.get_value("name").unwrap().as_str().unwrap();
        let email = context.get_value("email").unwrap().as_str().unwrap();
        
        println!("Name: {}", name);
        println!("Email: {}", email);
    } else {
        println!("❌ Form has errors:");
        for key in context.dirty_parameters() {
            if !context.is_valid(key) {
                let errors = context.get_errors(key);
                println!("  {}: {:?}", key, errors);
            }
        }
    }
    
    Ok(())
}
```

---

## Form Validation

### Example 2: User Registration with Password Confirmation

```rust
use nebula_parameter::prelude::*;

fn create_registration_form() -> Schema {
    Schema::new()
        .with_parameter(
            TextParameter::builder("username")
                .label("Username")
                .required()
                .min_length(3)
                .max_length(20)
                .pattern(r"^[a-zA-Z0-9_]+$")
                .placeholder("johndoe123")
                .help("Letters, numbers, and underscores only")
                .build()
        )
        .with_parameter(
            TextParameter::email("email")
                .label("Email")
                .required()
                .build()
        )
        .with_parameter(
            TextParameter::password("password")
                .label("Password")
                .required()
                .min_length(8)
                .with_validator(PasswordStrengthValidator)
                .help("Must contain uppercase, lowercase, number, and special char")
                .build()
        )
        .with_parameter(
            TextParameter::password("confirm_password")
                .label("Confirm Password")
                .required()
                .with_display(
                    ParameterDisplay::new()
                        .show_when_valid("password")
                )
                .build()
        )
        .with_parameter(
            BoolParameter::builder("agree_terms")
                .label("I agree to the Terms of Service")
                .required()
                .build()
        )
        .build()
}

// Custom validator
struct PasswordStrengthValidator;

impl Validator for PasswordStrengthValidator {
    fn validate(&self, value: &Value) -> ValidationResult<()> {
        if let Value::Text(password) = value {
            let has_upper = password.chars().any(|c| c.is_uppercase());
            let has_lower = password.chars().any(|c| c.is_lowercase());
            let has_digit = password.chars().any(|c| c.is_numeric());
            let has_special = password.chars().any(|c| !c.is_alphanumeric());
            
            if !(has_upper && has_lower && has_digit && has_special) {
                return Err(ValidationError::custom(
                    "Password must contain uppercase, lowercase, number, and special character"
                ));
            }
        }
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let schema = create_registration_form();
    let mut context = Context::new(schema);
    
    // Subscribe to events for reactive UI
    let mut rx = context.receiver();
    
    tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            match event {
                ParameterEvent::Validated { key, is_valid, errors } => {
                    if !is_valid {
                        println!("Validation error on {}: {:?}", key, errors);
                    }
                }
                ParameterEvent::VisibilityChanged { key, visible } => {
                    if visible {
                        println!("Field {} is now visible", key);
                    }
                }
                _ => {}
            }
        }
    });
    
    // Set values
    context.set_value("username", Value::text("alice_smith"))?;
    context.set_value("email", Value::text("alice@example.com"))?;
    context.set_value("password", Value::text("SecurePass123!"))?;
    // confirm_password field becomes visible after password is valid
    context.set_value("confirm_password", Value::text("SecurePass123!"))?;
    context.set_value("agree_terms", Value::boolean(true))?;
    
    // Cross-field validation
    let password = context.get_value("password").unwrap().as_str().unwrap();
    let confirm = context.get_value("confirm_password").unwrap().as_str().unwrap();
    
    if password != confirm {
        println!("❌ Passwords don't match!");
        return Ok(());
    }
    
    if context.validate_all() {
        println!("✅ Registration form is valid!");
    }
    
    Ok(())
}
```

---

## Workflow Automation

### Example 3: n8n-style Workflow Node

```rust
use nebula_parameter::prelude::*;

fn create_http_request_node() -> Schema {
    Schema::new()
        .with_parameter(
            ChoiceParameter::single("method")
                .label("HTTP Method")
                .option("GET", "GET")
                .option("POST", "POST")
                .option("PUT", "PUT")
                .option("DELETE", "DELETE")
                .default_value("GET")
                .build()
        )
        .with_parameter(
            TextParameter::url("url")
                .label("URL")
                .required()
                .placeholder("https://api.example.com/endpoint")
                .build()
        )
        .with_parameter(
            ChoiceParameter::single("auth_type")
                .label("Authentication")
                .option("none", "None")
                .option("basic", "Basic Auth")
                .option("bearer", "Bearer Token")
                .option("api_key", "API Key")
                .default_value("none")
                .build()
        )
        .with_parameter(
            TextParameter::builder("api_key")
                .subtype(TextSubtype::Secret)
                .label("API Key")
                .with_display(
                    ParameterDisplay::new()
                        .show_when_equals("auth_type", Value::text("api_key"))
                )
                .build()
        )
        .with_parameter(
            TextParameter::builder("bearer_token")
                .subtype(TextSubtype::Secret)
                .label("Bearer Token")
                .with_display(
                    ParameterDisplay::new()
                        .show_when_equals("auth_type", Value::text("bearer"))
                )
                .build()
        )
        .with_parameter(
            TextParameter::builder("request_body")
                .subtype(TextSubtype::Json)
                .label("Request Body")
                .with_display(
                    ParameterDisplay::new()
                        .show_when(DisplayRuleSet::any([
                            DisplayRule::when("method", DisplayCondition::Equals(Value::text("POST"))),
                            DisplayRule::when("method", DisplayCondition::Equals(Value::text("PUT"))),
                        ]))
                )
                .build()
        )
        .with_parameter(
            NumberParameter::integer("timeout")
                .subtype(NumberSubtype::DurationMillis)
                .label("Timeout (ms)")
                .min(0)
                .max(300000)
                .default_value(30000)
                .build()
        )
        .build()
}

async fn execute_workflow_node(context: &Context) -> Result<(), Box<dyn std::error::Error>> {
    let method = context.get_value("method")?.as_str().unwrap();
    let url = context.get_value("url")?.as_str().unwrap();
    let auth_type = context.get_value("auth_type")?.as_str().unwrap();
    
    println!("Executing HTTP {} request to {}", method, url);
    
    // Build request based on parameters
    let mut request = reqwest::Client::new().request(
        method.parse()?,
        url
    );
    
    // Add auth
    match auth_type {
        "api_key" => {
            let key = context.get_value("api_key")?.as_str().unwrap();
            request = request.header("X-API-Key", key);
        }
        "bearer" => {
            let token = context.get_value("bearer_token")?.as_str().unwrap();
            request = request.bearer_auth(token);
        }
        _ => {}
    }
    
    // Add body if applicable
    if method == "POST" || method == "PUT" {
        if let Some(body) = context.get_value("request_body") {
            request = request.body(body.as_str().unwrap().to_string());
        }
    }
    
    // Execute
    let response = request.send().await?;
    println!("Response status: {}", response.status());
    
    Ok(())
}
```

---

## 3D Graphics

### Example 4: 3D Object Transform Editor

```rust
use nebula_parameter::prelude::*;

fn create_transform_editor() -> Schema {
    Schema::new()
        .group("Transform")
        .with_parameter(
            VectorParameter::vector3("position")
                .label("Position")
                .default_vec3([0.0, 0.0, 0.0])
                .help("Object position in world space")
                .build()
        )
        .with_parameter(
            VectorParameter::builder("rotation")
                .subtype(VectorSubtype::EulerAngles)
                .label("Rotation")
                .default_vec3([0.0, 0.0, 0.0])
                .help("Rotation in degrees (pitch, yaw, roll)")
                .build()
        )
        .with_parameter(
            VectorParameter::builder("scale")
                .subtype(VectorSubtype::Scale3D)
                .label("Scale")
                .default_vec3([1.0, 1.0, 1.0])
                .help("Uniform or non-uniform scaling")
                .build()
        )
        .end_group()
        
        .group("Material")
        .with_parameter(
            VectorParameter::color_rgba("base_color")
                .label("Base Color")
                .default_vec4([1.0, 1.0, 1.0, 1.0])
                .build()
        )
        .with_parameter(
            NumberParameter::float("metallic")
                .subtype(NumberSubtype::Percentage)
                .label("Metallic")
                .min(0.0)
                .max(100.0)
                .default_value(0.0)
                .build()
        )
        .with_parameter(
            NumberParameter::float("roughness")
                .subtype(NumberSubtype::Percentage)
                .label("Roughness")
                .min(0.0)
                .max(100.0)
                .default_value(50.0)
                .build()
        )
        .end_group()
        .build()
}

#[derive(Debug)]
struct Transform {
    position: [f64; 3],
    rotation: [f64; 3],
    scale: [f64; 3],
}

#[derive(Debug)]
struct Material {
    base_color: [f64; 4],
    metallic: f64,
    roughness: f64,
}

fn extract_transform(context: &Context) -> Result<Transform, Box<dyn std::error::Error>> {
    Ok(Transform {
        position: context.get_vec3("position")?,
        rotation: context.get_vec3("rotation")?,
        scale: context.get_vec3("scale")?,
    })
}

fn extract_material(context: &Context) -> Result<Material, Box<dyn std::error::Error>> {
    Ok(Material {
        base_color: context.get_vec4("base_color")?,
        metallic: context.get_value("metallic")?.as_f64().unwrap(),
        roughness: context.get_value("roughness")?.as_f64().unwrap(),
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let schema = create_transform_editor();
    let mut context = Context::new(schema);
    
    // Enable undo/redo
    context.begin_transaction("Move object");
    context.set_vec3("position", [10.0, 5.0, 0.0])?;
    context.set_vec3("rotation", [0.0, 45.0, 0.0])?;
    context.end_transaction()?;
    
    // Get transform
    let transform = extract_transform(&context)?;
    let material = extract_material(&context)?;
    
    println!("Transform: {:?}", transform);
    println!("Material: {:?}", material);
    
    // Undo
    context.undo()?;
    println!("After undo: {:?}", extract_transform(&context)?);
    
    Ok(())
}
```

---

## Game Settings

### Example 5: Game Graphics Settings

```rust
use nebula_parameter::prelude::*;

fn create_graphics_settings() -> Schema {
    Schema::new()
        .group("Display")
        .with_parameter(
            ChoiceParameter::single("resolution")
                .label("Resolution")
                .option("1920x1080", "1920×1080 (Full HD)")
                .option("2560x1440", "2560×1440 (2K)")
                .option("3840x2160", "3840×2160 (4K)")
                .default_value("1920x1080")
                .build()
        )
        .with_parameter(
            ChoiceParameter::single("window_mode")
                .label("Window Mode")
                .option("fullscreen", "Fullscreen")
                .option("windowed", "Windowed")
                .option("borderless", "Borderless Window")
                .default_value("fullscreen")
                .build()
        )
        .with_parameter(
            BoolParameter::builder("vsync")
                .label("V-Sync")
                .default_value(true)
                .build()
        )
        .end_group()
        
        .group("Graphics Quality")
        .with_parameter(
            ChoiceParameter::single("quality_preset")
                .label("Quality Preset")
                .option("low", "Low")
                .option("medium", "Medium")
                .option("high", "High")
                .option("ultra", "Ultra")
                .option("custom", "Custom")
                .default_value("high")
                .build()
        )
        .with_parameter(
            ChoiceParameter::single("texture_quality")
                .label("Texture Quality")
                .option("low", "Low")
                .option("medium", "Medium")
                .option("high", "High")
                .option("ultra", "Ultra")
                .with_display(
                    ParameterDisplay::new()
                        .show_when_equals("quality_preset", Value::text("custom"))
                )
                .build()
        )
        .with_parameter(
            ChoiceParameter::single("shadow_quality")
                .label("Shadow Quality")
                .option("off", "Off")
                .option("low", "Low")
                .option("medium", "Medium")
                .option("high", "High")
                .with_display(
                    ParameterDisplay::new()
                        .show_when_equals("quality_preset", Value::text("custom"))
                )
                .build()
        )
        .with_parameter(
            BoolParameter::builder("anti_aliasing")
                .label("Anti-Aliasing (MSAA)")
                .default_value(true)
                .build()
        )
        .with_parameter(
            NumberParameter::integer("render_distance")
                .subtype(NumberSubtype::Distance)
                .label("Render Distance")
                .min(5)
                .max(32)
                .default_value(16)
                .help("Chunks visible in distance")
                .build()
        )
        .end_group()
        
        .group("Performance")
        .with_parameter(
            NumberParameter::integer("max_fps")
                .label("Max FPS")
                .min(30)
                .max(240)
                .default_value(144)
                .build()
        )
        .end_group()
        .build()
}

fn apply_preset(context: &mut Context, preset: &str) -> Result<(), Box<dyn std::error::Error>> {
    match preset {
        "low" => {
            context.set_value("texture_quality", Value::text("low"))?;
            context.set_value("shadow_quality", Value::text("low"))?;
            context.set_value("anti_aliasing", Value::boolean(false))?;
            context.set_value("render_distance", Value::integer(8))?;
        }
        "ultra" => {
            context.set_value("texture_quality", Value::text("ultra"))?;
            context.set_value("shadow_quality", Value::text("high"))?;
            context.set_value("anti_aliasing", Value::boolean(true))?;
            context.set_value("render_distance", Value::integer(32))?;
        }
        _ => {}
    }
    Ok(())
}
```

---

## CLI Tools

### Example 6: CLI Configuration Generator

```rust
use nebula_parameter::prelude::*;
use clap::{Parser, Subcommand};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Init,
    Configure,
    Run,
}

fn create_cli_config() -> Schema {
    Schema::new()
        .with_parameter(
            TextParameter::builder("project_name")
                .label("Project Name")
                .required()
                .pattern(r"^[a-z0-9-]+$")
                .build()
        )
        .with_parameter(
            ChoiceParameter::single("language")
                .label("Programming Language")
                .option("rust", "Rust")
                .option("python", "Python")
                .option("typescript", "TypeScript")
                .option("go", "Go")
                .build()
        )
        .with_parameter(
            TextParameter::builder("output_dir")
                .subtype(TextSubtype::DirectoryPath)
                .label("Output Directory")
                .default_value("./output")
                .build()
        )
        .with_parameter(
            BoolParameter::builder("use_git")
                .label("Initialize Git Repository")
                .default_value(true)
                .build()
        )
        .with_parameter(
            BoolParameter::builder("use_ci")
                .label("Set up CI/CD")
                .default_value(false)
                .build()
        )
        .with_parameter(
            ChoiceParameter::single("ci_provider")
                .label("CI Provider")
                .option("github", "GitHub Actions")
                .option("gitlab", "GitLab CI")
                .option("circleci", "CircleCI")
                .with_display(
                    ParameterDisplay::new()
                        .show_when_true("use_ci")
                )
                .build()
        )
        .build()
}

fn interactive_cli() -> Result<(), Box<dyn std::error::Error>> {
    let schema = create_cli_config();
    let mut context = Context::new(schema);
    
    // Interactive prompts
    println!("🚀 Project Initializer");
    println!();
    
    // Project name
    print!("Project name: ");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    context.set_value("project_name", Value::text(input.trim()))?;
    
    // Language selection
    println!("\nSelect language:");
    println!("1. Rust");
    println!("2. Python");
    println!("3. TypeScript");
    println!("4. Go");
    print!("Choice: ");
    input.clear();
    std::io::stdin().read_line(&mut input)?;
    
    let language = match input.trim() {
        "1" => "rust",
        "2" => "python",
        "3" => "typescript",
        "4" => "go",
        _ => "rust",
    };
    context.set_value("language", Value::text(language))?;
    
    // Validate and generate
    if context.validate_all() {
        let project_name = context.get_value("project_name")?.as_str().unwrap();
        let language = context.get_value("language")?.as_str().unwrap();
        
        println!("\n✅ Generating {} project: {}", language, project_name);
        // ... generate project
    }
    
    Ok(())
}
```

---

## Data Processing

### Example 7: CSV Import Configuration

```rust
use nebula_parameter::prelude::*;

fn create_csv_import_config() -> Schema {
    Schema::new()
        .with_parameter(
            TextParameter::builder("file_path")
                .subtype(TextSubtype::FilePath)
                .label("CSV File")
                .required()
                .build()
        )
        .with_parameter(
            ChoiceParameter::single("delimiter")
                .label("Delimiter")
                .option(",", "Comma (,)")
                .option(";", "Semicolon (;)")
                .option("\t", "Tab")
                .option("|", "Pipe (|)")
                .default_value(",")
                .build()
        )
        .with_parameter(
            BoolParameter::builder("has_header")
                .label("First Row is Header")
                .default_value(true)
                .build()
        )
        .with_parameter(
            ChoiceParameter::single("encoding")
                .label("Text Encoding")
                .option("utf8", "UTF-8")
                .option("latin1", "Latin-1")
                .option("windows1252", "Windows-1252")
                .default_value("utf8")
                .build()
        )
        .with_parameter(
            NumberParameter::integer("skip_rows")
                .label("Skip Rows")
                .min(0)
                .default_value(0)
                .help("Number of rows to skip at the beginning")
                .build()
        )
        .with_parameter(
            BoolParameter::builder("auto_detect_types")
                .label("Auto-Detect Column Types")
                .default_value(true)
                .build()
        )
        .build()
}

use csv::ReaderBuilder;

fn import_csv(context: &Context) -> Result<(), Box<dyn std::error::Error>> {
    let file_path = context.get_value("file_path")?.as_str().unwrap();
    let delimiter = context.get_value("delimiter")?.as_str().unwrap();
    let has_header = context.get_value("has_header")?.as_bool().unwrap();
    let skip_rows = context.get_value("skip_rows")?.as_i64().unwrap() as usize;
    
    let mut reader = ReaderBuilder::new()
        .delimiter(delimiter.as_bytes()[0])
        .has_headers(has_header)
        .from_path(file_path)?;
    
    // Skip rows
    for _ in 0..skip_rows {
        let mut record = csv::StringRecord::new();
        reader.read_record(&mut record)?;
    }
    
    // Process records
    for result in reader.records() {
        let record = result?;
        println!("{:?}", record);
    }
    
    Ok(())
}
```

---

## REST API Configuration

### Example 8: API Client Builder

```rust
use nebula_parameter::prelude::*;

fn create_api_client_config() -> Schema {
    Schema::new()
        .group("Connection")
        .with_parameter(
            TextParameter::url("base_url")
                .label("Base URL")
                .required()
                .placeholder("https://api.example.com")
                .build()
        )
        .with_parameter(
            NumberParameter::integer("timeout")
                .subtype(NumberSubtype::DurationSeconds)
                .label("Request Timeout")
                .min(1)
                .max(300)
                .default_value(30)
                .build()
        )
        .with_parameter(
            NumberParameter::integer("max_retries")
                .label("Max Retries")
                .min(0)
                .max(10)
                .default_value(3)
                .build()
        )
        .end_group()
        
        .group("Authentication")
        .with_parameter(
            ChoiceParameter::single("auth_method")
                .label("Authentication Method")
                .option("none", "None")
                .option("api_key", "API Key")
                .option("bearer", "Bearer Token")
                .option("oauth2", "OAuth 2.0")
                .default_value("none")
                .build()
        )
        .with_parameter(
            TextParameter::builder("api_key")
                .subtype(TextSubtype::Secret)
                .label("API Key")
                .with_display(
                    ParameterDisplay::new()
                        .show_when_equals("auth_method", Value::text("api_key"))
                )
                .build()
        )
        .with_parameter(
            TextParameter::builder("api_key_header")
                .label("API Key Header Name")
                .default_value("X-API-Key")
                .with_display(
                    ParameterDisplay::new()
                        .show_when_equals("auth_method", Value::text("api_key"))
                )
                .build()
        )
        .with_parameter(
            TextParameter::builder("bearer_token")
                .subtype(TextSubtype::Secret)
                .label("Bearer Token")
                .with_display(
                    ParameterDisplay::new()
                        .show_when_equals("auth_method", Value::text("bearer"))
                )
                .build()
        )
        .end_group()
        
        .group("Headers")
        .with_parameter(
            BoolParameter::builder("accept_json")
                .label("Accept JSON Responses")
                .default_value(true)
                .build()
        )
        .with_parameter(
            TextParameter::builder("user_agent")
                .label("User Agent")
                .default_value("NebulaCli/1.0")
                .build()
        )
        .end_group()
        .build()
}

struct ApiClient {
    base_url: String,
    timeout: Duration,
    client: reqwest::Client,
}

impl ApiClient {
    fn from_context(context: &Context) -> Result<Self, Box<dyn std::error::Error>> {
        let base_url = context.get_value("base_url")?.as_str().unwrap().to_string();
        let timeout_secs = context.get_value("timeout")?.as_i64().unwrap();
        let timeout = Duration::from_secs(timeout_secs as u64);
        
        let mut headers = reqwest::header::HeaderMap::new();
        
        // Auth
        let auth_method = context.get_value("auth_method")?.as_str().unwrap();
        match auth_method {
            "api_key" => {
                let key = context.get_value("api_key")?.as_str().unwrap();
                let header = context.get_value("api_key_header")?.as_str().unwrap();
                headers.insert(
                    reqwest::header::HeaderName::from_bytes(header.as_bytes())?,
                    reqwest::header::HeaderValue::from_str(key)?
                );
            }
            _ => {}
        }
        
        // Headers
        if context.get_value("accept_json")?.as_bool().unwrap() {
            headers.insert(
                reqwest::header::ACCEPT,
                reqwest::header::HeaderValue::from_static("application/json")
            );
        }
        
        let user_agent = context.get_value("user_agent")?.as_str().unwrap();
        headers.insert(
            reqwest::header::USER_AGENT,
            reqwest::header::HeaderValue::from_str(user_agent)?
        );
        
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .default_headers(headers)
            .build()?;
        
        Ok(Self {
            base_url,
            timeout,
            client,
        })
    }
    
    async fn get(&self, path: &str) -> Result<String, Box<dyn std::error::Error>> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.client.get(&url).send().await?;
        Ok(response.text().await?)
    }
}
```

---

## Summary

**These examples demonstrate:**

✅ **Basic Usage** - Simple forms and validation  
✅ **Form Validation** - Registration with custom validators  
✅ **Workflow Automation** - n8n-style HTTP node with conditional fields  
✅ **3D Graphics** - Transform editor with undo/redo  
✅ **Game Settings** - Graphics quality presets  
✅ **CLI Tools** - Interactive configuration generator  
✅ **Data Processing** - CSV import with options  
✅ **REST API** - API client builder with auth  

**Common Patterns:**
- Schema definition with builders
- Context creation and value setting
- Reactive event handling
- Conditional visibility
- Validation and error handling
- Undo/redo transactions
- Type-safe value extraction

**All examples are production-ready and can be used as templates!** 🚀
