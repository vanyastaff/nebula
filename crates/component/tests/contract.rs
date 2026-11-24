//! The shared component contract: typed keys per kind and base admission.

use nebula_component::{Component, ComponentKind, admit, key_of, kind};
use nebula_core::{ActionKey, CredentialKey, ResourceKey};
use nebula_metadata::{Metadata, MetadataBuildError, MetadataError, metadata_name};

struct Echo;
struct Pool;
struct Token;
struct Blank;

impl Component for Echo {
    type Kind = kind::Action;
    type Config = serde_json::Value;

    const KEY: &'static str = "echo";

    fn metadata() -> Metadata {
        Metadata::new(metadata_name!("Echo"), "Returns its input")
    }
}

impl Component for Pool {
    type Kind = kind::Resource;
    type Config = ();

    const KEY: &'static str = "pool";

    fn metadata() -> Metadata {
        Metadata::new(metadata_name!("Pool"), "A pooled client")
    }
}

impl Component for Token {
    type Kind = kind::Credential;
    type Config = ();

    const KEY: &'static str = "token";

    fn metadata() -> Metadata {
        Metadata::new(metadata_name!("Token"), "A static token")
    }
}

impl Component for Blank {
    type Kind = kind::Action;
    type Config = ();

    const KEY: &'static str = "blank";

    fn metadata() -> Metadata {
        Metadata::new(metadata_name!("Blank"), "Carries a blank tag").with_tags(["  "])
    }
}

#[test]
fn the_kind_chooses_the_key_type() {
    let action: ActionKey = key_of::<Echo>();
    let resource: ResourceKey = key_of::<Pool>();
    let credential: CredentialKey = key_of::<Token>();

    assert_eq!(action.as_str(), "echo");
    assert_eq!(resource.as_str(), "pool");
    assert_eq!(credential.as_str(), "token");
}

#[test]
fn kind_names_are_stable() {
    assert_eq!(<kind::Action as ComponentKind>::NAME, "action");
    assert_eq!(<kind::Resource as ComponentKind>::NAME, "resource");
    assert_eq!(<kind::Credential as ComponentKind>::NAME, "credential");
}

#[test]
fn admission_stamps_the_type_key_and_the_config_schema() {
    let definition = admit::<Pool>().expect("valid component");

    assert_eq!(definition.key(), &key_of::<Pool>());
    assert_eq!(definition.name(), "Pool");
    assert_eq!(
        definition.schema(),
        &nebula_schema::schema_of::<()>().expect("unit schema")
    );
}

#[test]
fn admission_rejects_metadata_that_breaks_a_catalog_rule() {
    let error = admit::<Blank>().expect_err("a blank tag is not admitted");

    assert!(matches!(
        error,
        MetadataBuildError::Metadata(MetadataError::BlankTag)
    ));
}
