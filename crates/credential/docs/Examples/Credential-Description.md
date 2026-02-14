---
title: Credential Description Example
tags: [nebula, nebula-credential, examples]
status: published
created: 2026-02-04
---

# Credential Description Example

This guide demonstrates how to define credential type schemas using `CredentialDescription`.

## Overview

`CredentialDescription` is used to define the **schema** of a credential type (not instances). It describes:
- What fields the credential requires
- Validation rules for each field
- UI metadata (icons, documentation)
- Parameter types and defaults

This enables:
- **Dynamic UI generation** - forms are generated from the schema
- **Type-safe validation** - user input validated against the schema
- **Self-documenting API** - schema serves as documentation
- **Credential registry** - centralized type definitions

## Basic Example: API Key

```rust
use nebula_credential::core::CredentialDescription;
use paramdef::{ParameterCollection, TextParameter, SecretParameter};

let api_key_type = CredentialDescription::builder()
    .key("api_key")
    .name("API Key")
    .description("Simple API key authentication")
    .icon("key")
    .properties(
        ParameterCollection::new()
            .with_parameter(
                SecretParameter::builder()
                    .key("api_key")
                    .name("API Key")
                    .description("The API key value")
                    .required(true)
                    .build()
            )
            .with_parameter(
                TextParameter::builder()
                    .key("header_name")
                    .name("Header Name")
                    .description("HTTP header name for the API key")
                    .default("Authorization")
                    .build()
            )
    )
    .build()
    .expect("Failed to build credential description");
```

## Advanced Example: GitHub OAuth2

```rust
use nebula_credential::core::CredentialDescription;
use paramdef::{
    ParameterCollection, 
    TextParameter, 
    SecretParameter, 
    ArrayParameter,
    SelectParameter,
};

let github_oauth2 = CredentialDescription::builder()
    .key("github_oauth2")
    .name("GitHub OAuth2")
    .description("OAuth2 authentication for GitHub API")
    .icon("github")
    .documentation_url("https://docs.github.com/en/apps/oauth-apps")
    .properties(
        ParameterCollection::new()
            .with_parameter(
                TextParameter::builder()
                    .key("client_id")
                    .name("Client ID")
                    .description("OAuth2 Client ID from GitHub")
                    .required(true)
                    .placeholder("Iv1.a1b2c3d4e5f6g7h8")
                    .build()
            )
            .with_parameter(
                SecretParameter::builder()
                    .key("client_secret")
                    .name("Client Secret")
                    .description("OAuth2 Client Secret from GitHub")
                    .required(true)
                    .build()
            )
            .with_parameter(
                TextParameter::builder()
                    .key("redirect_uri")
                    .name("Redirect URI")
                    .description("OAuth2 callback URL")
                    .default("http://localhost:8080/callback")
                    .required(true)
                    .build()
            )
            .with_parameter(
                ArrayParameter::builder()
                    .key("scopes")
                    .name("Scopes")
                    .description("OAuth2 scopes to request")
                    .default(vec!["repo".to_string(), "user".to_string()])
                    .options(vec![
                        "repo", "user", "gist", "notifications",
                        "read:org", "write:org", "admin:org"
                    ])
                    .build()
            )
            .with_parameter(
                SelectParameter::builder()
                    .key("flow_type")
                    .name("Flow Type")
                    .description("OAuth2 flow to use")
                    .default("authorization_code")
                    .options(vec![
                        ("authorization_code", "Authorization Code"),
                        ("client_credentials", "Client Credentials"),
                    ])
                    .build()
            )
    )
    .build()
    .expect("Failed to build GitHub OAuth2 description");
```

## Database Credential Example

```rust
use nebula_credential::core::CredentialDescription;
use paramdef::{
    ParameterCollection,
    TextParameter,
    SecretParameter,
    NumberParameter,
    BooleanParameter,
};

let postgres_type = CredentialDescription::builder()
    .key("postgres_db")
    .name("PostgreSQL Database")
    .description("PostgreSQL database connection credentials")
    .icon("database")
    .documentation_url("https://www.postgresql.org/docs/current/libpq-connect.html")
    .properties(
        ParameterCollection::new()
            .with_parameter(
                TextParameter::builder()
                    .key("host")
                    .name("Host")
                    .description("Database server hostname or IP address")
                    .default("localhost")
                    .required(true)
                    .build()
            )
            .with_parameter(
                NumberParameter::builder()
                    .key("port")
                    .name("Port")
                    .description("Database server port")
                    .default(5432)
                    .min(1)
                    .max(65535)
                    .required(true)
                    .build()
            )
            .with_parameter(
                TextParameter::builder()
                    .key("database")
                    .name("Database Name")
                    .description("Name of the database to connect to")
                    .required(true)
                    .build()
            )
            .with_parameter(
                TextParameter::builder()
                    .key("username")
                    .name("Username")
                    .description("Database username")
                    .required(true)
                    .build()
            )
            .with_parameter(
                SecretParameter::builder()
                    .key("password")
                    .name("Password")
                    .description("Database password")
                    .required(true)
                    .build()
            )
            .with_parameter(
                BooleanParameter::builder()
                    .key("ssl_enabled")
                    .name("Enable SSL")
                    .description("Use SSL/TLS for connection")
                    .default(true)
                    .build()
            )
    )
    .build()
    .expect("Failed to build PostgreSQL description");
```

## Serialization

Credential descriptions can be serialized to JSON for storage or API responses:

```rust
let json = serde_json::to_string_pretty(&github_oauth2)?;
println!("{}", json);
```

Output:
```json
{
  "key": "github_oauth2",
  "name": "GitHub OAuth2",
  "description": "OAuth2 authentication for GitHub API",
  "icon": "github",
  "documentation_url": "https://docs.github.com/en/apps/oauth-apps",
  "properties": {
    "parameters": [
      {
        "key": "client_id",
        "name": "Client ID",
        "type": "text",
        "required": true,
        "description": "OAuth2 Client ID from GitHub"
      },
      {
        "key": "client_secret",
        "name": "Client Secret",
        "type": "secret",
        "required": true,
        "description": "OAuth2 Client Secret from GitHub"
      }
    ]
  }
}
```

## Usage in Credential Registry

```rust
use std::collections::HashMap;
use nebula_credential::core::CredentialDescription;

pub struct CredentialRegistry {
    types: HashMap<String, CredentialDescription>,
}

impl CredentialRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            types: HashMap::new(),
        };
        
        // Register built-in types
        registry.register(create_api_key_type());
        registry.register(create_github_oauth2_type());
        registry.register(create_postgres_type());
        
        registry
    }
    
    pub fn register(&mut self, desc: CredentialDescription) {
        self.types.insert(desc.key.clone(), desc);
    }
    
    pub fn get(&self, key: &str) -> Option<&CredentialDescription> {
        self.types.get(key)
    }
    
    pub fn list(&self) -> Vec<&CredentialDescription> {
        self.types.values().collect()
    }
}

// Usage
let registry = CredentialRegistry::new();

// Get type definition for UI generation
if let Some(github_type) = registry.get("github_oauth2") {
    println!("Name: {}", github_type.name);
    println!("Parameters: {}", github_type.properties.len());
}

// List all available credential types
for cred_type in registry.list() {
    println!("- {} ({})", cred_type.name, cred_type.key);
}
```

## UI Integration

With `paramdef` integration, UIs can be automatically generated:

```rust
use paramdef::ui::generate_form;

// Get credential type
let github_type = registry.get("github_oauth2").unwrap();

// Generate UI form from parameters
let form = generate_form(&github_type.properties);

// Render in your UI framework (egui, web, etc.)
form.render(ui);
```

## Validation

Parameter definitions include validation rules:

```rust
use paramdef::validate;

let user_input = json!({
    "client_id": "Iv1.abc123",
    "client_secret": "secret_value",
    "scopes": ["repo", "user"]
});

// Validate against schema
match validate(&github_type.properties, &user_input) {
    Ok(validated) => {
        // Create credential instance with validated data
        create_credential("github_oauth2", validated).await?;
    }
    Err(errors) => {
        // Show validation errors to user
        for error in errors {
            eprintln!("Validation error: {}", error);
        }
    }
}
```

## Best Practices

1. **Use descriptive keys**: `github_oauth2` not `gh_auth`
2. **Provide clear descriptions**: Help users understand what each field is for
3. **Set sensible defaults**: Reduce friction for common use cases
4. **Include documentation URLs**: Link to official docs
5. **Use appropriate parameter types**: `SecretParameter` for passwords, `NumberParameter` for ports
6. **Validate thoroughly**: Use `paramdef` validation rules
7. **Icon consistency**: Use standard icon names across types

## See Also

- [[Core-Concepts|Core Concepts]] - Understanding credential types vs instances
- [[CredentialTypes|Built-in Credential Types]] - All available credential types
- [[How-To/Store-Credentials|Storing Credentials]] - Creating credential instances
- [paramdef documentation](https://docs.rs/paramdef) - Parameter definition system
