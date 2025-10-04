//! Credential Metadata Example
//!
//! Demonstrates how to use CredentialMetadata to discover available credential types
//! and their capabilities.

use nebula_credential::testing::TestCredentialFactory;
use nebula_credential::CredentialRegistry;
use std::sync::Arc;

fn main() {
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║          Credential Metadata Example                    ║");
    println!("╚══════════════════════════════════════════════════════════╝\n");

    // Create registry
    let registry = CredentialRegistry::new();

    // Register credential types
    println!("📦 Registering credential types...");
    registry.register(Arc::new(TestCredentialFactory::new()));
    println!("   ✓ Registered test_credential\n");

    // List all types
    println!("📋 Available credential types:");
    for type_name in registry.list_types() {
        println!("   • {}", type_name);
    }
    println!();

    // Get metadata for all types
    println!("📊 Credential Metadata:\n");
    for metadata in registry.list_metadata() {
        println!("╭─ {} ─────────────────────────────────", metadata.id);
        println!("│ Name: {}", metadata.name);
        println!("│ Description: {}", if metadata.description.is_empty() {
            "(none)"
        } else {
            metadata.description
        });
        println!("│ Supports Refresh: {}", if metadata.supports_refresh { "✓ Yes" } else { "✗ No" });
        println!("│ Requires User Interaction: {}", if metadata.requires_interaction { "✓ Yes" } else { "✗ No" });
        println!("╰────────────────────────────────────────────────\n");
    }

    // Get metadata for specific type
    println!("🔍 Looking up specific credential type...");
    if let Some(metadata) = registry.get_metadata("test_credential") {
        println!("   ✓ Found metadata for '{}'", metadata.id);
        println!("     └─ Supports refresh: {}", metadata.supports_refresh);
    }
    println!();

    // Check if type exists
    println!("✓ Checking type availability:");
    println!("   • test_credential exists: {}", registry.has_type("test_credential"));
    println!("   • oauth2 exists: {}", registry.has_type("oauth2"));
    println!();

    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║                    Summary                               ║");
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║ ✓ CredentialMetadata provides type discovery            ║");
    println!("║ ✓ list_metadata() returns all available types           ║");
    println!("║ ✓ get_metadata() retrieves specific type info           ║");
    println!("║ ✓ Metadata includes capabilities (refresh, interaction) ║");
    println!("╚══════════════════════════════════════════════════════════╝");

    println!("\n💡 Use Cases:");
    println!("   • Build dynamic UIs based on available credential types");
    println!("   • Validate credential type before creation");
    println!("   • Display user-friendly names and descriptions");
    println!("   • Check if a credential supports refresh before caching");
}
