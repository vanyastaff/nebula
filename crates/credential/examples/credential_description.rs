//! Example: Defining credential types using CredentialDescription
//!
//! This example demonstrates how to define credential type schemas
//! that can be used for type registry and documentation.

use nebula_credential::core::CredentialDescription;
use nebula_parameter::collection::ParameterCollection;
use nebula_parameter::def::ParameterDef;
use nebula_parameter::types::{SecretParameter, TextParameter};

fn main() {
    // Example 1: GitHub OAuth2 credential type (using builder)
    let github_properties = ParameterCollection::new()
        .with(ParameterDef::Text(TextParameter::new(
            "client_id",
            "Client ID",
        )))
        .with(ParameterDef::Secret(SecretParameter::new(
            "client_secret",
            "Client Secret",
        )));

    let github_oauth2 = CredentialDescription::builder()
        .key("github_oauth2")
        .name("GitHub OAuth2")
        .description("OAuth2 authentication for GitHub API")
        .icon("github")
        .documentation_url("https://docs.github.com/en/apps/oauth-apps")
        .properties(github_properties)
        .build()
        .expect("Failed to build GitHub OAuth2 credential description");

    println!("GitHub OAuth2 Credential Type:");
    println!("  Key: {}", github_oauth2.key);
    println!("  Name: {}", github_oauth2.name);
    println!("  Description: {}", github_oauth2.description);
    println!("  Icon: {:?}", github_oauth2.icon);
    println!("  Documentation: {:?}", github_oauth2.documentation_url);
    println!("  Properties: {} fields", github_oauth2.properties.len());
    println!();

    // Example 2: PostgreSQL database credential type (using builder)
    let postgres_properties = ParameterCollection::new()
        .with(ParameterDef::Text(TextParameter::new("host", "Host")))
        .with(ParameterDef::Text(TextParameter::new(
            "username", "Username",
        )))
        .with(ParameterDef::Secret(SecretParameter::new(
            "password", "Password",
        )));

    let postgres_db = CredentialDescription::builder()
        .key("postgres_db")
        .name("PostgreSQL Database")
        .description("PostgreSQL database connection credentials")
        .icon("database")
        .properties(postgres_properties)
        .build()
        .expect("Failed to build PostgreSQL credential description");

    println!("PostgreSQL Database Credential Type:");
    println!("  Key: {}", postgres_db.key);
    println!("  Name: {}", postgres_db.name);
    println!("  Description: {}", postgres_db.description);
    println!();

    // Example 3: Simple API Key credential type (direct construction)
    let api_key = CredentialDescription {
        key: "api_key".to_string(),
        name: "API Key".to_string(),
        description: "Simple API key authentication".to_string(),
        icon: Some("key".to_string()),
        icon_url: None,
        documentation_url: None,
        properties: ParameterCollection::new().with(ParameterDef::Secret(SecretParameter::new(
            "api_key", "API Key",
        ))),
    };

    println!("API Key Credential Type:");
    println!("  Key: {}", api_key.key);
    println!("  Name: {}", api_key.name);
    println!();

    // Serialization example
    println!("Serialized GitHub OAuth2:");
    let json = serde_json::to_string_pretty(&github_oauth2).expect("Failed to serialize");
    println!("{json}");
}
