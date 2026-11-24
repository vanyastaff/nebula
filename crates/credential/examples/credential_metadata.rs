//! Example: define, admit, record, and re-admit credential metadata.

#![expect(
    clippy::print_stdout,
    reason = "example: printed output is the demonstration"
)]

use nebula_credential::{
    ApiKeyCredential, Component, CredentialRegistry, RecordedCredentialDefinition,
};

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut registry = CredentialRegistry::new();
    registry.register(ApiKeyCredential, "credential-metadata-example")?;
    let admitted = registry
        .definition(ApiKeyCredential::KEY)
        .expect("the successfully registered credential must have metadata");

    println!("Admitted key: {}", admitted.base().key().as_str());
    println!("Admitted auth pattern: {:?}", admitted.pattern());
    println!("Admitted name: {}", admitted.base().name());
    println!(
        "Canonical schema fields: {}",
        admitted.base().schema().properties().len()
    );

    let wire = serde_json::to_vec(admitted)?;
    let recorded = RecordedCredentialDefinition::from_slice(&wire, Default::default())?;
    let readmitted = recorded.readmit_against(admitted)?;
    println!("Readmitted version: {}", readmitted.base().version());

    Ok(())
}
