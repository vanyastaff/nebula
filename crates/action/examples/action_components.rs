//! Example demonstrating ActionComponents for declaring action dependencies.

use nebula_action::ActionComponents;
use nebula_credential::CredentialRef;
use nebula_resource::ResourceRef;

// Example credential types
struct GithubToken;
struct SlackWebhook;

// Example resource types
struct PostgresDb;
struct RedisCache;

fn main() {
    // Create components with builder pattern
    let components = ActionComponents::new()
        .credential(CredentialRef::of::<GithubToken>())
        .credential(CredentialRef::of::<SlackWebhook>())
        .resource(ResourceRef::of::<PostgresDb>())
        .resource(ResourceRef::of::<RedisCache>());

    println!("Action dependencies:");
    println!("  Credentials: {}", components.credentials().len());
    println!("  Resources: {}", components.resources().len());
    println!("  Total: {}", components.len());
    println!("  Is empty: {}", components.is_empty());

    // Using batch methods
    let components2 = ActionComponents::new()
        .with_credentials(vec![
            CredentialRef::of::<GithubToken>(),
            CredentialRef::of::<SlackWebhook>(),
        ])
        .with_resources(vec![
            ResourceRef::of::<PostgresDb>(),
            ResourceRef::of::<RedisCache>(),
        ]);

    println!("\nComponents created with batch methods:");
    println!("  Credentials: {}", components2.credentials().len());
    println!("  Resources: {}", components2.resources().len());

    // Destructuring
    let (creds, resources) = components.into_parts();
    println!("\nDestructured components:");
    println!("  Credentials: {}", creds.len());
    println!("  Resources: {}", resources.len());
}
