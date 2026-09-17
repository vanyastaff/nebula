use nebula_core::credential_key;
use nebula_metadata::{
    BaseCompatError, CatalogCategoryKey, CatalogLink, CatalogLinkRelation, DeprecationNotice, Icon,
    MaturityLevel,
};
use semver::Version;

use super::{CredentialMetadata, CredentialMetadataDraft, MetadataCompatibilityError};
use crate::AuthPattern;

fn admitted(pattern: AuthPattern, major: u64, minor: u64) -> CredentialMetadata {
    CredentialMetadataDraft::new(
        credential_key!("cred"),
        crate::metadata_name!("Credential"),
        "description",
    )
    .with_version(Version::new(major, minor, 0))
    .admit_with_schema(
        nebula_schema::schema_of::<()>().expect("unit schema is valid"),
        pattern,
    )
    .expect("valid credential metadata")
}

#[test]
fn draft_fields_survive_admission() {
    let version = Version::parse("2.1.3-beta.1").expect("version literal is valid");
    let category: CatalogCategoryKey = "auth.tokens".parse().expect("valid category");
    let link = CatalogLink::new(
        CatalogLinkRelation::Setup,
        "/credentials/cred/setup"
            .parse()
            .expect("valid relative link"),
    );
    let documentation_link = CatalogLink::new(
        CatalogLinkRelation::Overview,
        "https://example.test/credentials/cred"
            .parse()
            .expect("valid documentation link"),
    );
    let metadata = CredentialMetadataDraft::new(
        credential_key!("cred"),
        crate::metadata_name!("Credential"),
        "description",
    )
    .with_version(version.clone())
    .with_icon(Icon::None)
    .with_inline_icon("key")
    .with_documentation_url("https://example.test/credentials/cred")
    .with_categories([category.clone()])
    .add_link(link.clone())
    .with_tags(["credential"])
    .add_tag("catalog")
    .mark_beta()
    .admit_with_schema(
        nebula_schema::schema_of::<()>().expect("unit schema is valid"),
        AuthPattern::SecretToken,
    )
    .expect("valid credential metadata");

    assert_eq!(metadata.version(), &version);
    assert_eq!(metadata.icon(), &Icon::inline("key"));
    assert_eq!(
        metadata.documentation_url(),
        Some("https://example.test/credentials/cred")
    );
    assert_eq!(metadata.categories(), std::slice::from_ref(&category));
    assert_eq!(metadata.links(), [documentation_link, link]);
    assert_eq!(metadata.tags(), ["catalog", "credential"]);
    assert_eq!(metadata.maturity(), MaturityLevel::Beta);
    assert_eq!(metadata.pattern(), AuthPattern::SecretToken);
}

#[test]
fn draft_icon_methods_preserve_curated_variants() {
    let inline = CredentialMetadataDraft::new(
        credential_key!("inline"),
        crate::metadata_name!("Inline"),
        "description",
    )
    .with_inline_icon("key")
    .admit_with_schema(
        nebula_schema::schema_of::<()>().expect("unit schema is valid"),
        AuthPattern::SecretToken,
    )
    .expect("valid credential metadata");
    let url = CredentialMetadataDraft::new(
        credential_key!("url"),
        crate::metadata_name!("URL"),
        "description",
    )
    .with_url_icon("https://example.test/icon.svg")
    .admit_with_schema(
        nebula_schema::schema_of::<()>().expect("unit schema is valid"),
        AuthPattern::SecretToken,
    )
    .expect("valid credential metadata");
    let none = CredentialMetadataDraft::new(
        credential_key!("none"),
        crate::metadata_name!("None"),
        "description",
    )
    .with_icon(Icon::None)
    .admit_with_schema(
        nebula_schema::schema_of::<()>().expect("unit schema is valid"),
        AuthPattern::SecretToken,
    )
    .expect("valid credential metadata");

    assert_eq!(inline.icon(), &Icon::inline("key"));
    assert_eq!(url.icon(), &Icon::url("https://example.test/icon.svg"));
    assert_eq!(none.icon(), &Icon::None);
}

#[test]
fn draft_lifecycle_and_tags_survive_admission() {
    let experimental = CredentialMetadataDraft::new(
        credential_key!("experimental"),
        crate::metadata_name!("Experimental"),
        "description",
    )
    .mark_experimental()
    .admit_with_schema(
        nebula_schema::schema_of::<()>().expect("unit schema is valid"),
        AuthPattern::SecretToken,
    )
    .expect("valid credential metadata");
    let beta = CredentialMetadataDraft::new(
        credential_key!("beta"),
        crate::metadata_name!("Beta"),
        "description",
    )
    .mark_experimental()
    .mark_beta()
    .admit_with_schema(
        nebula_schema::schema_of::<()>().expect("unit schema is valid"),
        AuthPattern::SecretToken,
    )
    .expect("valid credential metadata");
    let stable = CredentialMetadataDraft::new(
        credential_key!("stable"),
        crate::metadata_name!("Stable"),
        "description",
    )
    .mark_beta()
    .mark_stable()
    .admit_with_schema(
        nebula_schema::schema_of::<()>().expect("unit schema is valid"),
        AuthPattern::SecretToken,
    )
    .expect("valid credential metadata");
    let notice = DeprecationNotice::new(Version::new(2, 0, 0))
        .with_replacement(nebula_metadata::CatalogReference::credential(
            credential_key!("replacement"),
        ))
        .with_reason("superseded");
    let deprecated = CredentialMetadataDraft::new(
        credential_key!("deprecated"),
        crate::metadata_name!("Deprecated"),
        "description",
    )
    .with_tags(["auth"])
    .add_tag("legacy")
    .with_version(Version::new(2, 0, 0))
    .with_deprecation(notice.clone())
    .mark_stable()
    .admit_with_schema(
        nebula_schema::schema_of::<()>().expect("unit schema is valid"),
        AuthPattern::SecretToken,
    )
    .expect("valid credential metadata");

    assert_eq!(experimental.maturity(), MaturityLevel::Experimental);
    assert_eq!(beta.maturity(), MaturityLevel::Beta);
    assert_eq!(stable.maturity(), MaturityLevel::Stable);
    assert_eq!(deprecated.maturity(), MaturityLevel::Deprecated);
    assert_eq!(deprecated.deprecation(), Some(&notice));
    assert_eq!(deprecated.tags(), ["auth", "legacy"]);
}

#[test]
fn admitted_wire_nests_versioned_base_and_records_as_evidence() {
    let metadata = admitted(AuthPattern::SecretToken, 2, 1);
    let encoded = serde_json::to_string(&metadata).expect("metadata serializes");
    let value: serde_json::Value =
        serde_json::from_str(&encoded).expect("serialized metadata is valid JSON");
    assert!(value.get("key").is_none());
    assert_eq!(value["base"]["metadata_wire_version"], 2);
    assert_eq!(
        value["base"].get("key").and_then(serde_json::Value::as_str),
        Some("cred")
    );

    let recorded: super::RecordedCredentialMetadata =
        serde_json::from_str(&encoded).expect("metadata records as evidence");
    assert_eq!(recorded.readmit_against(&metadata), Ok(metadata));
}

#[test]
fn historical_schema_cannot_admit_credential_metadata() {
    let historical: nebula_schema::ValidSchema = serde_json::from_str(r#"{"fields":[]}"#).unwrap();
    let result = CredentialMetadataDraft::new(
        credential_key!("cred"),
        crate::metadata_name!("Credential"),
        "description",
    )
    .admit_with_schema(historical, AuthPattern::SecretToken);
    std::assert_matches!(
        result,
        Err(super::CredentialMetadataAdmissionError::CatalogMetadata)
    );
}

#[test]
fn credential_readmission_requires_the_same_schema_policy() {
    let current = admitted(AuthPattern::SecretToken, 1, 0);
    let mut wire = serde_json::to_value(&current).unwrap();
    wire["base"]["schema"]
        .as_object_mut()
        .unwrap()
        .remove("policy_version");
    let recorded: super::RecordedCredentialMetadata = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(&recorded).unwrap(), wire);
    assert!(recorded.readmit_against(&current).is_err());
}

#[test]
fn pattern_change_requires_major_bump() {
    let previous = admitted(AuthPattern::SecretToken, 1, 0);
    let next = admitted(AuthPattern::OAuth2, 1, 1);
    assert_eq!(
        next.validate_compatibility(&previous),
        Err(MetadataCompatibilityError::PatternChangeWithoutMajorBump)
    );
}

#[test]
fn pattern_change_with_major_is_accepted() {
    let previous = admitted(AuthPattern::SecretToken, 1, 0);
    let next = admitted(AuthPattern::OAuth2, 2, 0);
    assert!(next.validate_compatibility(&previous).is_ok());
}

#[test]
fn key_change_is_rejected() {
    let previous = admitted(AuthPattern::SecretToken, 1, 0);
    let next = CredentialMetadataDraft::new(
        credential_key!("other"),
        crate::metadata_name!("Credential"),
        "description",
    )
    .admit_with_schema(
        nebula_schema::schema_of::<()>().expect("unit schema is valid"),
        AuthPattern::SecretToken,
    )
    .expect("valid credential metadata");
    assert_eq!(
        next.validate_compatibility(&previous),
        Err(MetadataCompatibilityError::Base(
            BaseCompatError::KeyChanged {
                previous: credential_key!("cred"),
                current: credential_key!("other"),
            }
        ))
    );
}
