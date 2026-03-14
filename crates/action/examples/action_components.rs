//! Example demonstrating ActionComponents for declaring action dependencies.

use async_trait::async_trait;
use nebula_action::ActionComponents;
use nebula_core::ResourceKey;
use nebula_credential::CredentialRef;
use nebula_credential::core::result::InitializeResult;
use nebula_credential::core::{CredentialContext, CredentialDescription};
use nebula_credential::traits::CredentialType;
use nebula_parameter::schema::Schema;
use nebula_resource::ResourceRef;
use nebula_resource::context::Context;
use nebula_resource::metadata::ResourceMetadata;
use nebula_resource::resource::Resource;

// Example credential types
struct GithubToken;
struct SlackWebhook;

#[async_trait]
impl CredentialType for GithubToken {
    type Input = ();
    type State = nebula_credential::protocols::ApiKeyState;
    fn description() -> CredentialDescription {
        CredentialDescription::builder()
            .key("github_token")
            .name("GitHub Token")
            .description("")
            .properties(Schema::new())
            .build()
            .unwrap()
    }
    async fn initialize(
        &self,
        _: &(),
        _: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, nebula_credential::core::CredentialError> {
        unreachable!()
    }
}

#[async_trait]
impl CredentialType for SlackWebhook {
    type Input = ();
    type State = nebula_credential::protocols::ApiKeyState;
    fn description() -> CredentialDescription {
        CredentialDescription::builder()
            .key("slack_webhook")
            .name("Slack Webhook")
            .description("")
            .properties(Schema::new())
            .build()
            .unwrap()
    }
    async fn initialize(
        &self,
        _: &(),
        _: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, nebula_credential::core::CredentialError> {
        unreachable!()
    }
}

// Example resource types
struct PostgresDb;
struct RedisCache;

struct ExampleResourceConfig;
impl nebula_resource::resource::Config for ExampleResourceConfig {}

impl Resource for PostgresDb {
    type Config = ExampleResourceConfig;
    type Instance = ();
    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::from_key(ResourceKey::try_from("postgres_db").unwrap())
    }
    async fn create(
        &self,
        _: &ExampleResourceConfig,
        _: &Context,
    ) -> nebula_resource::Result<()> {
        Ok(())
    }
}

impl Resource for RedisCache {
    type Config = ExampleResourceConfig;
    type Instance = ();
    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::from_key(ResourceKey::try_from("redis_cache").unwrap())
    }
    async fn create(
        &self,
        _: &ExampleResourceConfig,
        _: &Context,
    ) -> nebula_resource::Result<()> {
        Ok(())
    }
}

fn main() {
    // Create components with builder pattern
    let components = ActionComponents::new()
        .credential(CredentialRef::<GithubToken>::of())
        .credential(CredentialRef::<SlackWebhook>::of())
        .resource(ResourceRef::<PostgresDb>::of())
        .resource(ResourceRef::<RedisCache>::of());

    println!("Action dependencies:");
    println!("  Credentials: {}", components.credentials().len());
    println!("  Resources: {}", components.resources().len());
    println!("  Total: {}", components.len());
    println!("  Is empty: {}", components.is_empty());

    // Using batch methods
    let components2 = ActionComponents::new()
        .with_credentials(vec![
            CredentialRef::<GithubToken>::of().erase(),
            CredentialRef::<SlackWebhook>::of().erase(),
        ])
        .with_resources(vec![
            ResourceRef::<PostgresDb>::of().erase(),
            ResourceRef::<RedisCache>::of().erase(),
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
