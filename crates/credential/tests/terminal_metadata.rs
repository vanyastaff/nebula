use nebula_credential::{
    ApiKeyCredential, AuthPattern, Credential, CredentialMetadata, CredentialMetadataDraft,
    CredentialRegistry, RecordedCredentialMetadata,
};
use nebula_schema::schema_of;

#[test]
fn registry_admits_the_properties_schema_into_immutable_metadata() {
    let draft = ApiKeyCredential::metadata();
    assert_eq!(draft.pattern(), AuthPattern::SecretToken);

    let mut registry = CredentialRegistry::new();
    registry
        .register(ApiKeyCredential, "terminal-metadata-test")
        .expect("the built-in definition is valid");

    let admitted = registry
        .metadata(ApiKeyCredential::KEY)
        .expect("registered metadata is available");
    assert_eq!(admitted.key().as_str(), ApiKeyCredential::KEY);
    assert_eq!(
        admitted.schema(),
        &schema_of::<<ApiKeyCredential as Credential>::Properties>()
            .expect("built-in properties schema is valid"),
    );
}

#[test]
fn recorded_metadata_is_evidence_readmitted_against_a_fresh_definition() {
    let mut registry = CredentialRegistry::new();
    registry
        .register(ApiKeyCredential, "terminal-metadata-test")
        .expect("the built-in definition is valid");
    let fresh = registry
        .metadata(ApiKeyCredential::KEY)
        .expect("registered metadata is available");

    let wire = serde_json::to_value(fresh).expect("admitted metadata serializes");
    let recorded: RecordedCredentialMetadata =
        serde_json::from_value(wire).expect("wire metadata records as evidence");
    let readmitted: CredentialMetadata = recorded
        .readmit_against(fresh)
        .expect("matching evidence readmits the fresh definition");

    assert_eq!(&readmitted, fresh);
}

#[test]
fn static_metadata_draft_construction_is_infallible() {
    let _: CredentialMetadataDraft = ApiKeyCredential::metadata();
}
