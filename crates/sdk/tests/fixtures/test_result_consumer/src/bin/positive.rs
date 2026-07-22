use nebula_sdk::integration::credential::{TestFailureCode, TestResult};

fn main() {
    let result = TestResult::Failed {
        code: TestFailureCode::AuthenticationRejected,
    };
    let TestResult::Failed { code } = &result else {
        panic!("constructed failure must preserve its stable code");
    };
    assert_eq!(*code, TestFailureCode::AuthenticationRejected);
    assert_eq!(
        result.failure_code(),
        Some(TestFailureCode::AuthenticationRejected)
    );
}
