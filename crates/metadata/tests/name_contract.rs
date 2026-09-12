//! Static proofs and dynamic validation share the catalog-name contract.

use nebula_core::{action_key, credential_key, resource_key};
use nebula_metadata::{MetadataDraft, MetadataError, MetadataName, metadata_name};
use nebula_schema::ValidSchema;
use proptest::prelude::*;

#[test]
fn literals_carry_a_nonblank_name_proof() {
    const NAME: MetadataName = metadata_name!("  Example  ");
    let metadata = MetadataDraft::new(action_key!("example"), NAME, "")
        .bind_schema(ValidSchema::empty())
        .expect("valid bounded metadata");
    assert_eq!(metadata.name(), "  Example  ");
    assert_eq!(metadata.key(), &action_key!("example"));
}

#[test]
fn static_and_dynamic_names_reject_unicode_whitespace() {
    for name in [
        "",
        " \t\r\n",
        "\u{85}\u{a0}\u{1680}\u{2000}\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}",
    ] {
        assert_eq!(
            MetadataName::from_static(name),
            Err(MetadataError::BlankName)
        );
        assert_eq!(MetadataName::try_from(name), Err(MetadataError::BlankName));
        serde_json::from_value::<MetadataName>(serde_json::json!(name)).expect_err("blank name");
    }
    const NAME: MetadataName = metadata_name!("\u{0418}\u{043c}\u{044f}");
    assert_eq!(NAME.as_str(), "\u{0418}\u{043c}\u{044f}");
}

#[test]
fn typed_keys_can_supply_default_display_names() {
    assert_eq!(
        MetadataName::from(action_key!("example.action")).as_str(),
        "example.action"
    );
    assert_eq!(
        MetadataName::from(credential_key!("example.credential")).as_str(),
        "example.credential"
    );
    assert_eq!(
        MetadataName::from(resource_key!("example.resource")).as_str(),
        "example.resource"
    );
}

proptest! {
    #[test]
    fn checked_names_preserve_text_and_round_trip(name in any::<String>()) {
        let checked = MetadataName::try_from(name.clone());
        if name.trim().is_empty() {
            prop_assert_eq!(checked, Err(MetadataError::BlankName));
        } else {
            let checked = checked?;
            prop_assert_eq!(checked.as_str(), &name);
            let decoded: MetadataName = serde_json::from_value(serde_json::json!(checked))?;
            prop_assert_eq!(decoded, checked);
        }
    }
}
