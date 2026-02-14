use nebula_config::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔧 Simple Configuration Example");

    // Create a basic configuration from environment
    let _config = ConfigBuilder::new()
        .with_source(ConfigSource::Env)
        .build()
        .await?;

    println!("✅ Configuration created successfully");

    // Show some configuration sources
    println!("\n📋 Configuration source types:");
    let sources = vec![
        ConfigSource::Env,
        ConfigSource::File("config.json".into()),
        ConfigSource::Default,
    ];

    for source in sources {
        println!("   - {}: priority {}", source.name(), source.priority());
    }

    Ok(())
}
