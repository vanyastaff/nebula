//! Tests for the v2 Credential derive macro - successful cases.

use nebula_macros::Credential;
include!("support.rs");

/// A stub auth scheme for testing.
#[derive(Clone, Debug)]
pub struct TestScheme {
    pub token: String,
}

/// A stub static protocol for testing.
pub struct TestProtocol;

impl StaticProtocol for TestProtocol {
    type Scheme = TestScheme;

    fn parameters() -> collection::ParameterCollection {
        collection::ParameterCollection::new()
    }

    fn build(
        values: &values::ParameterValues,
    ) -> Result<TestScheme, core::CredentialError> {
        let _ = values;
        Ok(TestScheme {
            token: "test".to_owned(),
        })
    }
}

/// Basic credential with required attributes only.
#[derive(Credential)]
#[credential(
    key = "test_basic",
    name = "Test Basic",
    scheme = TestScheme,
    protocol = TestProtocol,
)]
pub struct BasicCredential;

/// Credential with optional icon attribute.
#[derive(Credential)]
#[credential(
    key = "test_icon",
    name = "Test Icon",
    scheme = TestScheme,
    protocol = TestProtocol,
    icon = "key",
)]
pub struct IconCredential;

/// Credential with all optional attributes.
#[derive(Credential)]
#[credential(
    key = "test_full",
    name = "Test Full",
    scheme = TestScheme,
    protocol = TestProtocol,
    icon = "database",
    doc_url = "https://example.com/docs",
)]
pub struct FullCredential;

fn main() {
    // Verify KEY const
    assert_eq!(BasicCredential::KEY, "test_basic");
    assert_eq!(IconCredential::KEY, "test_icon");
    assert_eq!(FullCredential::KEY, "test_full");

    // Verify description
    let desc = BasicCredential::description();
    assert_eq!(desc.key, "test_basic");
    assert_eq!(desc.name, "Test Basic");
    assert!(desc.icon.is_none());
    assert!(desc.documentation_url.is_none());

    let desc = IconCredential::description();
    assert_eq!(desc.icon, Some("key".to_owned()));

    let desc = FullCredential::description();
    assert_eq!(desc.icon, Some("database".to_owned()));
    assert_eq!(
        desc.documentation_url,
        Some("https://example.com/docs".to_owned())
    );

    // Verify project (identity path)
    let scheme = TestScheme {
        token: "hello".to_owned(),
    };
    let projected = BasicCredential::project(&scheme);
    assert_eq!(projected.token, "hello");
}
