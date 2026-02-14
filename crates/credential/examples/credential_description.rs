//! Example: Defining credential types using CredentialDescription
//!
//! This example demonstrates how to define credential type schemas
//! that can be used for UI generation, validation, and documentation.

use nebula_credential::core::CredentialDescription;
use paramdef::{ArrayParameter, ParameterCollection, SecretParameter, TextParameter};

fn main() {
    // Example 1: GitHub OAuth2 credential type
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
                        .build(),
                )
                .with_parameter(
                    SecretParameter::builder()
                        .key("client_secret")
                        .name("Client Secret")
                        .description("OAuth2 Client Secret from GitHub")
                        .required(true)
                        .build(),
                )
                .with_parameter(
                    ArrayParameter::builder()
                        .key("scopes")
                        .name("Scopes")
                        .description("OAuth2 scopes to request (e.g., 'repo', 'user')")
                        .default(vec!["repo".to_string(), "user".to_string()])
                        .build(),
                ),
        )
        .build()
        .expect("Failed to build GitHub OAuth2 credential description");

    println!("GitHub OAuth2 Credential Type:");
    println!("  Key: {}", github_oauth2.key);
    println!("  Name: {}", github_oauth2.name);
    println!("  Description: {}", github_oauth2.description);
    println!("  Icon: {:?}", github_oauth2.icon);
    println!("  Documentation: {:?}", github_oauth2.documentation_url);
    println!("  Parameters: {} defined", github_oauth2.properties.len());
    println!();

    // Example 2: PostgreSQL database credential type
    let postgres_db = CredentialDescription::builder()
        .key("postgres_db")
        .name("PostgreSQL Database")
        .description("PostgreSQL database connection credentials")
        .icon("database")
        .properties(
            ParameterCollection::new()
                .with_parameter(
                    TextParameter::builder()
                        .key("host")
                        .name("Host")
                        .description("Database host address")
                        .default("localhost")
                        .required(true)
                        .build(),
                )
                .with_parameter(
                    TextParameter::builder()
                        .key("port")
                        .name("Port")
                        .description("Database port")
                        .default("5432")
                        .required(true)
                        .build(),
                )
                .with_parameter(
                    TextParameter::builder()
                        .key("database")
                        .name("Database Name")
                        .description("Name of the database")
                        .required(true)
                        .build(),
                )
                .with_parameter(
                    TextParameter::builder()
                        .key("username")
                        .name("Username")
                        .description("Database username")
                        .required(true)
                        .build(),
                )
                .with_parameter(
                    SecretParameter::builder()
                        .key("password")
                        .name("Password")
                        .description("Database password")
                        .required(true)
                        .build(),
                ),
        )
        .build()
        .expect("Failed to build PostgreSQL credential description");

    println!("PostgreSQL Database Credential Type:");
    println!("  Key: {}", postgres_db.key);
    println!("  Name: {}", postgres_db.name);
    println!("  Description: {}", postgres_db.description);
    println!("  Parameters: {} defined", postgres_db.properties.len());
    println!();

    // Example 3: Simple API Key credential type
    let api_key = CredentialDescription {
        key: "api_key".to_string(),
        name: "API Key".to_string(),
        description: "Simple API key authentication".to_string(),
        icon: Some("key".to_string()),
        icon_url: None,
        documentation_url: None,
        properties: ParameterCollection::new()
            .with_parameter(
                SecretParameter::builder()
                    .key("api_key")
                    .name("API Key")
                    .description("The API key value")
                    .required(true)
                    .build(),
            )
            .with_parameter(
                TextParameter::builder()
                    .key("header_name")
                    .name("Header Name")
                    .description("HTTP header name for the API key")
                    .default("Authorization")
                    .build(),
            ),
    };

    println!("API Key Credential Type:");
    println!("  Key: {}", api_key.key);
    println!("  Name: {}", api_key.name);
    println!("  Parameters: {} defined", api_key.properties.len());
    println!();

    // Serialization example
    println!("Serialized GitHub OAuth2:");
    let json = serde_json::to_string_pretty(&github_oauth2).expect("Failed to serialize");
    println!("{}", json);
}
