use nebula_parameter::{Parameters, rules::Rule};

#[derive(Parameters)]
#[allow(dead_code)]
struct ValidatedInput {
    #[param(label = "URL")]
    #[validate(required, url)]
    url: String,

    #[param(label = "Name")]
    #[validate(min_length = 1, max_length = 100)]
    name: String,

    #[param(label = "Port")]
    #[validate(min = 1, max = 65535)]
    port: u32,

    #[param(label = "Code")]
    #[validate(pattern = r"^[A-Z]{3}$")]
    code: Option<String>,
}

#[test]
fn derive_validate_attaches_rules() {
    let params = ValidatedInput::parameters();

    let url_param = params.get("url").expect("url param");
    assert!(url_param.required);
    assert!(
        url_param
            .rules
            .iter()
            .any(|r| matches!(r, Rule::Url { .. }))
    );

    let name_param = params.get("name").expect("name param");
    assert!(
        name_param
            .rules
            .iter()
            .any(|r| matches!(r, Rule::MinLength { min: 1, .. }))
    );
    assert!(
        name_param
            .rules
            .iter()
            .any(|r| matches!(r, Rule::MaxLength { max: 100, .. }))
    );

    let port_param = params.get("port").expect("port param");
    assert!(
        !port_param.rules.is_empty(),
        "port should have min/max rules"
    );

    let code_param = params.get("code").expect("code param");
    assert!(
        code_param
            .rules
            .iter()
            .any(|r| matches!(r, Rule::Pattern { .. }))
    );
}
