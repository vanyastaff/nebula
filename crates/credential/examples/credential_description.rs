//! Example: Defining credential types using CredentialDescription
//!
//! This example demonstrates how to define credential type schemas
//! that can be used for type registry and documentation.

use nebula_credential::core::CredentialDescription;
use nebula_parameter::schema::{Field, Schema};

fn main() {
    // Example 1: GitHub OAuth2 credential type
    let github_properties = Schema::new()
        .field(Field::text("client_id").with_label("Client ID").required())
        .field(
            Field::text("client_secret")
                .with_label("Client Secret")
                .required()
                .secret(),
        );

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

    // Example 2: PostgreSQL database credential type
    let postgres_properties = Schema::new()
        .field(Field::text("host").with_label("Host").required())
        .field(Field::text("username").with_label("Username").required())
        .field(
            Field::text("password")
                .with_label("Password")
                .required()
                .secret(),
        );

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
        properties: Schema::new().field(
            Field::text("api_key")
                .with_label("API Key")
                .required()
                .secret(),
        ),
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

