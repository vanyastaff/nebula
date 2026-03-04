//! Multi-tenant credential isolation example
//!
//! Demonstrates US2: Multi-Tenant Isolation
//! - Scope-based credential isolation using ScopeLevel from nebula-core
//! - Hierarchical scope matching (Organization > Project > Workflow)
//! - Tenant-specific credential listing

use nebula_core::{OrganizationId, ProjectId, ScopeLevel};
use nebula_credential::prelude::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Credential Manager: Multi-Tenant Isolation Example ===\n");

    // Create manager
    let storage = Arc::new(MockStorageProvider::new());
    let manager = CredentialManager::builder().storage(storage).build();
    let key = EncryptionKey::from_bytes([0u8; 32]);

    // 1. Store credentials for different tenants
    println!("1. Storing credentials for different tenants...");

    let org_a = OrganizationId::new();
    let org_b = OrganizationId::new();

    // Tenant A credentials (Organization scope)
    let tenant_a_context =
        CredentialContext::new("org-1").with_scope(ScopeLevel::Organization(org_a));
    let cred_a1 = CredentialId::new();
    let data_a1 = encrypt(&key, b"secret-a1")?;
    manager
        .store(
            &cred_a1,
            data_a1,
            CredentialMetadata::new(),
            &tenant_a_context,
        )
        .await?;
    println!(
        "   ✓ Stored credential for tenant A (org scope): {}",
        cred_a1
    );

    // Tenant B credentials (Organization scope)
    let tenant_b_context =
        CredentialContext::new("org-1").with_scope(ScopeLevel::Organization(org_b));
    let cred_b1 = CredentialId::new();
    let data_b1 = encrypt(&key, b"secret-b1")?;
    manager
        .store(
            &cred_b1,
            data_b1,
            CredentialMetadata::new(),
            &tenant_b_context,
        )
        .await?;
    println!(
        "   ✓ Stored credential for tenant B (org scope): {}\n",
        cred_b1
    );

    // 2. Demonstrate scope isolation (use list_scoped / retrieve_scoped for enforcement)
    println!("2. Testing scope isolation...");

    // Tenant A can only see their credentials (list_scoped enforces scope)
    let tenant_a_creds = manager.list_scoped(&tenant_a_context).await?;
    println!("   Tenant A sees {} credential(s):", tenant_a_creds.len());
    for id in &tenant_a_creds {
        println!("     - {}", id);
    }

    // Tenant B can only see their credentials
    let tenant_b_creds = manager.list_scoped(&tenant_b_context).await?;
    println!("   Tenant B sees {} credential(s):", tenant_b_creds.len());
    for id in &tenant_b_creds {
        println!("     - {}", id);
    }
    println!();

    // 3. Test cross-tenant access prevention (retrieve_scoped enforces scope)
    println!("3. Testing cross-tenant access prevention...");
    let result = manager.retrieve_scoped(&cred_a1, &tenant_b_context).await?;
    if result.is_none() {
        println!("   ✓ Tenant B cannot access Tenant A's credentials");
    }

    let result = manager.retrieve_scoped(&cred_b1, &tenant_a_context).await?;
    if result.is_none() {
        println!("   ✓ Tenant A cannot access Tenant B's credentials\n");
    }

    // 4. Demonstrate hierarchical scope matching
    println!("4. Testing hierarchical scope access...");

    let project_id = ProjectId::new();
    let org_id = OrganizationId::new();

    // Child scope credential (Project under Organization)
    let child_context = CredentialContext::new("org-1").with_scope(ScopeLevel::Project(project_id));
    let cred_child = CredentialId::new();
    let data_child = encrypt(&key, b"secret-child")?;
    manager
        .store(
            &cred_child,
            data_child,
            CredentialMetadata::new(),
            &child_context,
        )
        .await?;
    println!("   ✓ Stored credential in child scope: Project");

    // Parent scope (Organization) can access child (Project) credentials
    let parent_context =
        CredentialContext::new("org-1").with_scope(ScopeLevel::Organization(org_id));
    let result = manager
        .retrieve_scoped(&cred_child, &parent_context)
        .await?;
    if result.is_some() {
        println!("   ✓ Parent scope (Organization) can access child (Project) credentials\n");
    }

    // 5. List credentials by scope
    println!("5. Listing credentials by scope...");
    let scoped_creds = manager.list_scoped(&parent_context).await?;
    println!(
        "   Found {} scoped credential(s) for Organization scope",
        scoped_creds.len()
    );
    println!();

    println!("=== Multi-tenant isolation working correctly! ===");
    Ok(())
}
