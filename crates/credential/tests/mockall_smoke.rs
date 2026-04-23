//! [`mockall`] example for this crate — use for **traits under test**, not for `reqwest` (use
//! `wiremock`).

use mockall::automock;
use pretty_assertions::assert_eq;

/// Test-only seam (real code uses concrete types; this shows the pattern).
#[automock]
pub trait TestSecretVersion {
    fn version_label(&self) -> String;
}

#[test]
fn mockall_returns_configured_version() {
    let mut mock = MockTestSecretVersion::new();
    mock.expect_version_label()
        .times(1)
        .returning(|| "v-test".to_owned());

    assert_eq!(mock.version_label(), "v-test");
}
