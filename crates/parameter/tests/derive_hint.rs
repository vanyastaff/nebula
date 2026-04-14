//! Tests that `#[param(hint = "...")]` generates `.input_hint(InputHint::...)` calls
//! rather than the deprecated `.input_type("...")` string form.

use nebula_parameter::{InputHint, ParameterType};

#[derive(nebula_parameter::Parameters)]
#[allow(dead_code)]
// Fields exist only to exercise `#[param(hint = "...")]` attribute
// expansion. The test asserts on generated parameter metadata, not on
// field values, so the fields are never read after struct construction.
struct HintTestInput {
    #[param(label = "Email", hint = "email")]
    email: String,

    #[param(label = "Start Date", hint = "date")]
    start_date: String,

    #[param(label = "Website", hint = "url")]
    website: String,
}

#[test]
fn derive_hint_uses_input_hint_enum() {
    let params = HintTestInput::parameters();

    let email_param = params.get("email").expect("email param");
    match &email_param.param_type {
        ParameterType::String { input_hint, .. } => {
            assert_eq!(*input_hint, InputHint::Email);
        },
        other => panic!("expected String, got {:?}", other),
    }

    let date_param = params.get("start_date").expect("start_date param");
    match &date_param.param_type {
        ParameterType::String { input_hint, .. } => {
            assert_eq!(*input_hint, InputHint::Date);
        },
        other => panic!("expected String, got {:?}", other),
    }

    let url_param = params.get("website").expect("website param");
    match &url_param.param_type {
        ParameterType::String { input_hint, .. } => {
            assert_eq!(*input_hint, InputHint::Url);
        },
        other => panic!("expected String, got {:?}", other),
    }
}
