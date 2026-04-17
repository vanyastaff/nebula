//! Example: Defining credential types using CredentialMetadata
//!
//! This example demonstrates how to define credential type schemas
//! that can be used for type registry and documentation.

use nebula_credential::CredentialMetadata;
use nebula_parameter::{Parameter, ParameterCollection};

fn main() {
    // Example 1: GitHub OAuth2 credential type
    let github_properties = ParameterCollection::new()
        .add(Parameter::string("client_id").label("Client ID").required())
        .add(
            Parameter::string("client_secret")
                .label("Client Secret")
                .required()
                .secret(),
        );

    let github_oauth2 = CredentialMetadata::builder()
        .key("github_oauth2")
        .name("GitHub OAuth2")
        .description("OAuth2 authentication for GitHub API")
        .icon("github")
        .documentation_url("https://docs.github.com/en/apps/oauth-apps")
        .properties(github_properties)
        .pattern(nebula_core::AuthPattern::OAuth2)
        .build()
        .expect("Failed to build GitHub OAuth2 credential metadata");

    println!("GitHub OAuth2 Credential Type:");
    println!("  Key: {}", github_oauth2.key);
    println!("  Name: {}", github_oauth2.name);
    println!("  Description: {}", github_oauth2.description);
    println!("  Icon: {:?}", github_oauth2.icon);
    println!("  Documentation: {:?}", github_oauth2.documentation_url);
    println!("  Properties: {} fields", github_oauth2.properties.len());
    println!();

    // Example 2: PostgreSQL database credential type
    let postgres_properties = ParameterCollection::new()
        .add(Parameter::string("host").label("Host").required())
        .add(Parameter::string("username").label("Username").required())
        .add(
            Parameter::string("password")
                .label("Password")
                .required()
                .secret(),
        );

    let postgres_db = CredentialMetadata::builder()
        .key("postgres_db")
        .name("PostgreSQL Database")
        .description("PostgreSQL database connection credentials")
        .icon("database")
        .properties(postgres_properties)
        .pattern(nebula_core::AuthPattern::IdentityPassword)
        .build()
        .expect("Failed to build PostgreSQL credential metadata");

    println!("PostgreSQL Database Credential Type:");
    println!("  Key: {}", postgres_db.key);
    println!("  Name: {}", postgres_db.name);
    println!("  Description: {}", postgres_db.description);
    println!();

    // Example 3: Simple API Key credential type (direct construction)
    let api_key = CredentialMetadata {
        key: "api_key".to_string(),
        name: "API Key".to_string(),
        description: "Simple API key authentication".to_string(),
        icon: Some("key".to_string()),
        icon_url: None,
        documentation_url: None,
        properties: ParameterCollection::new().add(
            Parameter::string("api_key")
                .label("API Key")
                .required()
                .secret(),
        ),
        pattern: nebula_core::AuthPattern::SecretToken,
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
