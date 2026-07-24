//! A provider's raw failure text must not be constructible as a `TestResult`.

use nebula_credential::TestResult;

fn main() {
    let provider_reason = String::from("provider echoed bearer token");
    let _ = TestResult::Failed {
        reason: provider_reason,
    };
}
