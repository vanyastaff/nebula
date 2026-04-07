use nebula_parameter::Parameters;

#[derive(Parameters, serde::Deserialize, Debug, PartialEq)]
#[serde(default)]
struct DefaultAlignedInput {
    #[param(default = "GET")]
    method: String,

    #[param(default = 30)]
    timeout: u32,

    #[param(default = true)]
    verbose: bool,

    // No default — should use Default::default()
    name: String,
}

#[test]
fn serde_defaults_align_with_param_defaults() {
    let json = serde_json::json!({});
    let input: DefaultAlignedInput = serde_json::from_value(json).unwrap();
    assert_eq!(input.method, "GET");
    assert_eq!(input.timeout, 30);
    assert!(input.verbose);
    assert_eq!(input.name, ""); // Default for String
}

#[test]
fn partial_values_use_defaults_for_missing() {
    let json = serde_json::json!({"name": "test"});
    let input: DefaultAlignedInput = serde_json::from_value(json).unwrap();
    assert_eq!(input.method, "GET");
    assert_eq!(input.name, "test");
}
