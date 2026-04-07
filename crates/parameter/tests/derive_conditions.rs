use nebula_parameter::{HasParameters, Parameters};

#[derive(Parameters)]
struct ConditionalInput {
    method: String,

    #[param(
        label = "Body",
        visible_when_field = "method",
        visible_when_value = "POST"
    )]
    body: Option<String>,

    #[param(
        label = "Token",
        required_when_field = "method",
        required_when_value = "oauth2"
    )]
    token: Option<String>,
}

#[test]
fn derive_condition_attributes() {
    let params = ConditionalInput::parameters();

    let body_param = params.get("body").expect("body param");
    assert!(body_param.visible_when.is_some(), "body should have visible_when");

    let token_param = params.get("token").expect("token param");
    assert!(token_param.required_when.is_some(), "token should have required_when");
}
