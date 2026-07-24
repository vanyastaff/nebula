use nebula_sdk::integration::credential::TestResult;

fn main() {
    let provider_reason = String::from("provider echoed bearer token");
    let _ = TestResult::Failed {
        reason: provider_reason,
    };
}
