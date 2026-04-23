//! Example: [`mockall`] for local test doubles — use for traits you own in a test module,
//! not for large external surfaces (prefer `wiremock` for HTTP).

use mockall::automock;
use pretty_assertions::assert_eq;

/// Tiny clock seam — shows `expect_*` + `returning` without pulling production code.
#[automock]
pub trait TestClock {
    /// Milliseconds since an arbitrary epoch.
    fn now_ms(&self) -> u64;
}

#[test]
fn mockall_clock_returns_configured_value() {
    let mut mock = MockTestClock::new();
    mock.expect_now_ms()
        .times(1)
        .returning(|| 1_700_000_000_000);

    assert_eq!(mock.now_ms(), 1_700_000_000_000);
}
