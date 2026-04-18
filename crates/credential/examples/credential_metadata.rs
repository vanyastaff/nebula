//! Example: Defining credential types using `CredentialMetadata`.
//!
//! Builds on the shared [`nebula_metadata::BaseMetadata`] prefix — the
//! catalog-level fields (`key`, `name`, `description`, `schema`, `icon`,
//! `documentation_url`, `tags`, `maturity`, `deprecation`) are uniform
//! across action/credential/resource. Credential-specific details
//! (`pattern`) stay on [`CredentialMetadata`] itself.

use nebula_credential::CredentialMetadata;
use nebula_metadata::Metadata;
use nebula_schema::{Field, Schema};

fn main() {
    // Example 1: GitHub OAuth2 credential type
    let github_schema = Schema::builder()
        .add(Field::string("client_id").label("Client ID").required())
        .add(
            Field::secret("client_secret")
                .label("Client Secret")
                .required(),
        )
        .build()
        .expect("github schema is always valid");

    let github_oauth2 = CredentialMetadata::builder()
        .key(nebula_core::credential_key!("github_oauth2"))
        .name("GitHub OAuth2")
        .description("OAuth2 authentication for GitHub API")
        .icon("github")
        .documentation_url("https://docs.github.com/en/apps/oauth-apps")
        .schema(github_schema)
        .pattern(nebula_core::AuthPattern::OAuth2)
        .build()
        .expect("Failed to build GitHub OAuth2 credential metadata");

    println!("GitHub OAuth2 Credential Type:");
    println!("  Key: {}", github_oauth2.key().as_str());
    println!("  Name: {}", github_oauth2.name());
    println!("  Description: {}", github_oauth2.description());
    println!("  Icon: {:?}", github_oauth2.icon());
    println!("  Documentation: {:?}", github_oauth2.documentation_url());
    println!("  Schema fields: {}", github_oauth2.schema().fields().len());
    println!();

    // Example 2: PostgreSQL database credential type
    let postgres_schema = Schema::builder()
        .add(Field::string("host").label("Host").required())
        .add(Field::string("username").label("Username").required())
        .add(Field::secret("password").label("Password").required())
        .build()
        .expect("postgres schema is always valid");

    let postgres_db = CredentialMetadata::builder()
        .key(nebula_core::credential_key!("postgres_db"))
        .name("PostgreSQL Database")
        .description("PostgreSQL database connection credentials")
        .icon("database")
        .schema(postgres_schema)
        .pattern(nebula_core::AuthPattern::IdentityPassword)
        .build()
        .expect("Failed to build PostgreSQL credential metadata");

    println!("PostgreSQL Database Credential Type:");
    println!("  Key: {}", postgres_db.key().as_str());
    println!("  Name: {}", postgres_db.name());
    println!("  Description: {}", postgres_db.description());
    println!();

    // Example 3: Simple API Key credential type via the `new` constructor.
    let api_key_schema = Schema::builder()
        .add(Field::secret("api_key").label("API Key").required())
        .build()
        .expect("api_key schema is always valid");

    let api_key = CredentialMetadata::new(
        nebula_core::credential_key!("api_key"),
        "API Key",
        "Simple API key authentication",
        api_key_schema,
        nebula_core::AuthPattern::SecretToken,
    );

    println!("API Key Credential Type:");
    println!("  Key: {}", api_key.key().as_str());
    println!("  Name: {}", api_key.name());
    println!();

    // Serialization example
    println!("Serialized GitHub OAuth2:");
    let json = serde_json::to_string_pretty(&github_oauth2).expect("Failed to serialize");
    println!("{json}");
}
