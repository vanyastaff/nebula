//! Example: define, admit, record, and re-admit credential metadata.

#![expect(
    clippy::print_stdout,
    reason = "example: printed output is the demonstration"
)]

use nebula_credential::{
    ApiKeyCredential, Credential, CredentialRegistry, RecordedCredentialMetadata,
};

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let draft = ApiKeyCredential::metadata();
    println!("Draft auth pattern: {:?}", draft.pattern());

    let mut registry = CredentialRegistry::new();
    registry.register(ApiKeyCredential, "credential-metadata-example")?;
    let admitted = registry
        .metadata(ApiKeyCredential::KEY)
        .expect("the successfully registered credential must have metadata");

    println!("Admitted key: {}", admitted.key().as_str());
    println!("Admitted name: {}", admitted.name());
    println!(
        "Canonical schema fields: {}",
        admitted.schema().fields().len()
    );

    let wire = serde_json::to_value(admitted)?;
    let recorded: RecordedCredentialMetadata = serde_json::from_value(wire)?;
    let readmitted = recorded.readmit_against(admitted)?;
    println!("Readmitted version: {}", readmitted.version());

    Ok(())
}
