//! Integration tests for the display condition system

use nebula_parameter::prelude::*;
use nebula_value::Value;

#[test]
fn test_complex_display_conditions() {
    // Show API key field when auth_type is "api_key" AND advanced mode is enabled
    let display = ParameterDisplay::new().show_when(DisplayRuleSet::all([
        DisplayRule::when(
            ParameterKey::new("auth_type").unwrap(),
            DisplayCondition::Equals(Value::text("api_key")),
        ),
        DisplayRule::when(
            ParameterKey::new("advanced_mode").unwrap(),
            DisplayCondition::IsTrue,
        ),
    ]));

    let ctx1 = DisplayContext::new()
        .with_value(
            ParameterKey::new("auth_type").unwrap(),
            Value::text("api_key"),
        )
        .with_value(
            ParameterKey::new("advanced_mode").unwrap(),
            Value::boolean(true),
        );

    let ctx2 = DisplayContext::new()
        .with_value(
            ParameterKey::new("auth_type").unwrap(),
            Value::text("api_key"),
        )
        .with_value(
            ParameterKey::new("advanced_mode").unwrap(),
            Value::boolean(false),
        );

    assert!(display.should_display(&ctx1));
    assert!(!display.should_display(&ctx2));
}

#[test]
fn test_display_dependencies() {
    let display = ParameterDisplay::new()
        .show_when(DisplayRule::when(
            ParameterKey::new("auth_type").unwrap(),
            DisplayCondition::Equals(Value::text("api_key")),
        ))
        .hide_when(DisplayRule::when(
            ParameterKey::new("disabled").unwrap(),
            DisplayCondition::IsTrue,
        ));

    let deps = display.dependencies();
    assert!(deps.contains(&ParameterKey::new("auth_type").unwrap()));
    assert!(deps.contains(&ParameterKey::new("disabled").unwrap()));
}

#[test]
fn test_display_serialization() {
    let display = ParameterDisplay::new()
        .show_when_equals(ParameterKey::new("mode").unwrap(), Value::text("advanced"));

    let json = serde_json::to_string(&display).unwrap();
    let restored: ParameterDisplay = serde_json::from_str(&json).unwrap();

    assert_eq!(display, restored);
}

#[test]
fn test_text_parameter_with_display_condition() {
    let param = TextParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("api_key")
                .name("API Key")
                .description("Enter your API key")
                .build()
                .unwrap(),
        )
        .display(ParameterDisplay::new().show_when_equals(
            ParameterKey::new("auth_type").unwrap(),
            Value::text("api_key"),
        ))
        .build();

    let ctx_show = DisplayContext::new().with_value(
        ParameterKey::new("auth_type").unwrap(),
        Value::text("api_key"),
    );

    let ctx_hide = DisplayContext::new().with_value(
        ParameterKey::new("auth_type").unwrap(),
        Value::text("oauth"),
    );

    assert!(param.should_display(&ctx_show));
    assert!(!param.should_display(&ctx_hide));
}

#[test]
fn test_number_parameter_with_range_display() {
    let param = NumberParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("timeout")
                .name("Timeout")
                .description("Request timeout in seconds")
                .build()
                .unwrap(),
        )
        .display(ParameterDisplay::new().show_when(DisplayRule::when(
            ParameterKey::new("advanced_mode").unwrap(),
            DisplayCondition::IsTrue,
        )))
        .build();

    let ctx_show = DisplayContext::new().with_value(
        ParameterKey::new("advanced_mode").unwrap(),
        Value::boolean(true),
    );

    let ctx_hide = DisplayContext::new().with_value(
        ParameterKey::new("advanced_mode").unwrap(),
        Value::boolean(false),
    );

    assert!(param.should_display(&ctx_show));
    assert!(!param.should_display(&ctx_hide));
}

#[test]
fn test_checkbox_parameter_with_not_condition() {
    let param = CheckboxParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("enable_ssl")
                .name("Enable SSL")
                .description("Use SSL connection")
                .build()
                .unwrap(),
        )
        .display(
            ParameterDisplay::new().show_when(DisplayRuleSet::not(DisplayRule::when(
                ParameterKey::new("use_http").unwrap(),
                DisplayCondition::IsTrue,
            ))),
        )
        .build();

    let ctx_show = DisplayContext::new().with_value(
        ParameterKey::new("use_http").unwrap(),
        Value::boolean(false),
    );

    let ctx_hide = DisplayContext::new()
        .with_value(ParameterKey::new("use_http").unwrap(), Value::boolean(true));

    assert!(param.should_display(&ctx_show));
    assert!(!param.should_display(&ctx_hide));
}

#[test]
fn test_select_parameter_with_or_condition() {
    let param = SelectParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("region")
                .name("Region")
                .description("Select region")
                .build()
                .unwrap(),
        )
        .options(vec![]) // Required field
        .display(ParameterDisplay::new().show_when(DisplayRuleSet::any([
            DisplayRule::when(
                ParameterKey::new("service").unwrap(),
                DisplayCondition::Equals(Value::text("s3")),
            ),
            DisplayRule::when(
                ParameterKey::new("service").unwrap(),
                DisplayCondition::Equals(Value::text("ec2")),
            ),
        ])))
        .build();

    let ctx_s3 =
        DisplayContext::new().with_value(ParameterKey::new("service").unwrap(), Value::text("s3"));

    let ctx_ec2 =
        DisplayContext::new().with_value(ParameterKey::new("service").unwrap(), Value::text("ec2"));

    let ctx_other = DisplayContext::new()
        .with_value(ParameterKey::new("service").unwrap(), Value::text("lambda"));

    assert!(param.should_display(&ctx_s3));
    assert!(param.should_display(&ctx_ec2));
    assert!(!param.should_display(&ctx_other));
}

#[test]
fn test_parameter_collection_with_display_dependencies() {
    let mut collection = ParameterCollection::new();

    // Auth type selector
    let auth_type_param = SelectParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("auth_type")
                .name("Authentication Type")
                .description("Select authentication method")
                .build()
                .unwrap(),
        )
        .options(vec![]) // Required field
        .build();

    // API key field (shown when auth_type is "api_key")
    let api_key_param = TextParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("api_key")
                .name("API Key")
                .description("Enter your API key")
                .build()
                .unwrap(),
        )
        .display(ParameterDisplay::new().show_when_equals(
            ParameterKey::new("auth_type").unwrap(),
            Value::text("api_key"),
        ))
        .build();

    // OAuth client ID (shown when auth_type is "oauth")
    let oauth_client_id_param = TextParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("oauth_client_id")
                .name("OAuth Client ID")
                .description("Enter your OAuth client ID")
                .build()
                .unwrap(),
        )
        .display(ParameterDisplay::new().show_when_equals(
            ParameterKey::new("auth_type").unwrap(),
            Value::text("oauth"),
        ))
        .build();

    collection.add(auth_type_param);
    collection.add(api_key_param);
    collection.add(oauth_client_id_param);

    // Check dependencies
    let api_key: &TextParameter = collection
        .get(ParameterKey::new("api_key").unwrap())
        .expect("api_key parameter should exist");
    let deps = api_key.dependencies();
    assert!(deps.contains(&ParameterKey::new("auth_type").unwrap()));
}

#[test]
fn test_nested_display_conditions() {
    // Show field when (auth_type == "api_key" AND advanced_mode == true) OR superuser == true
    let display = ParameterDisplay::new().show_when(DisplayRuleSet::any([
        DisplayRuleSet::all([
            DisplayRule::when(
                ParameterKey::new("auth_type").unwrap(),
                DisplayCondition::Equals(Value::text("api_key")),
            ),
            DisplayRule::when(
                ParameterKey::new("advanced_mode").unwrap(),
                DisplayCondition::IsTrue,
            ),
        ]),
        DisplayRuleSet::single(DisplayRule::when(
            ParameterKey::new("superuser").unwrap(),
            DisplayCondition::IsTrue,
        )),
    ]));

    // Case 1: Both conditions in AND are met
    let ctx1 = DisplayContext::new()
        .with_value(
            ParameterKey::new("auth_type").unwrap(),
            Value::text("api_key"),
        )
        .with_value(
            ParameterKey::new("advanced_mode").unwrap(),
            Value::boolean(true),
        )
        .with_value(
            ParameterKey::new("superuser").unwrap(),
            Value::boolean(false),
        );

    // Case 2: Only superuser condition met
    let ctx2 = DisplayContext::new()
        .with_value(
            ParameterKey::new("auth_type").unwrap(),
            Value::text("oauth"),
        )
        .with_value(
            ParameterKey::new("advanced_mode").unwrap(),
            Value::boolean(false),
        )
        .with_value(
            ParameterKey::new("superuser").unwrap(),
            Value::boolean(true),
        );

    // Case 3: Neither OR branch is satisfied
    let ctx3 = DisplayContext::new()
        .with_value(
            ParameterKey::new("auth_type").unwrap(),
            Value::text("api_key"),
        )
        .with_value(
            ParameterKey::new("advanced_mode").unwrap(),
            Value::boolean(false),
        )
        .with_value(
            ParameterKey::new("superuser").unwrap(),
            Value::boolean(false),
        );

    assert!(display.should_display(&ctx1));
    assert!(display.should_display(&ctx2));
    assert!(!display.should_display(&ctx3));
}

#[test]
fn test_string_conditions() {
    // Contains
    let display_contains = ParameterDisplay::new().show_when(DisplayRule::when(
        ParameterKey::new("url").unwrap(),
        DisplayCondition::Contains("https".to_string()),
    ));

    let ctx_https = DisplayContext::new().with_value(
        ParameterKey::new("url").unwrap(),
        Value::text("https://example.com"),
    );
    let ctx_http = DisplayContext::new().with_value(
        ParameterKey::new("url").unwrap(),
        Value::text("http://example.com"),
    );

    assert!(display_contains.should_display(&ctx_https));
    assert!(!display_contains.should_display(&ctx_http));

    // StartsWith
    let display_starts = ParameterDisplay::new().show_when(DisplayRule::when(
        ParameterKey::new("filename").unwrap(),
        DisplayCondition::StartsWith("config".to_string()),
    ));

    let ctx_config = DisplayContext::new().with_value(
        ParameterKey::new("filename").unwrap(),
        Value::text("config.json"),
    );
    let ctx_data = DisplayContext::new().with_value(
        ParameterKey::new("filename").unwrap(),
        Value::text("data.json"),
    );

    assert!(display_starts.should_display(&ctx_config));
    assert!(!display_starts.should_display(&ctx_data));

    // EndsWith
    let display_ends = ParameterDisplay::new().show_when(DisplayRule::when(
        ParameterKey::new("filename").unwrap(),
        DisplayCondition::EndsWith(".json".to_string()),
    ));

    let ctx_json = DisplayContext::new().with_value(
        ParameterKey::new("filename").unwrap(),
        Value::text("data.json"),
    );
    let ctx_yaml = DisplayContext::new().with_value(
        ParameterKey::new("filename").unwrap(),
        Value::text("data.yaml"),
    );

    assert!(display_ends.should_display(&ctx_json));
    assert!(!display_ends.should_display(&ctx_yaml));
}

#[test]
fn test_numeric_conditions() {
    // GreaterThan
    let display_gt = ParameterDisplay::new().show_when(DisplayRule::when(
        ParameterKey::new("count").unwrap(),
        DisplayCondition::GreaterThan(10.0),
    ));

    let ctx_15 =
        DisplayContext::new().with_value(ParameterKey::new("count").unwrap(), Value::integer(15));
    let ctx_5 =
        DisplayContext::new().with_value(ParameterKey::new("count").unwrap(), Value::integer(5));

    assert!(display_gt.should_display(&ctx_15));
    assert!(!display_gt.should_display(&ctx_5));

    // LessThan
    let display_lt = ParameterDisplay::new().show_when(DisplayRule::when(
        ParameterKey::new("count").unwrap(),
        DisplayCondition::LessThan(100.0),
    ));

    let ctx_50 =
        DisplayContext::new().with_value(ParameterKey::new("count").unwrap(), Value::integer(50));
    let ctx_150 =
        DisplayContext::new().with_value(ParameterKey::new("count").unwrap(), Value::integer(150));

    assert!(display_lt.should_display(&ctx_50));
    assert!(!display_lt.should_display(&ctx_150));

    // InRange
    let display_range = ParameterDisplay::new().show_when(DisplayRule::when(
        ParameterKey::new("percentage").unwrap(),
        DisplayCondition::InRange {
            min: 0.0,
            max: 100.0,
        },
    ));

    let ctx_50_pct = DisplayContext::new()
        .with_value(ParameterKey::new("percentage").unwrap(), Value::integer(50));
    let ctx_150_pct = DisplayContext::new().with_value(
        ParameterKey::new("percentage").unwrap(),
        Value::integer(150),
    );

    assert!(display_range.should_display(&ctx_50_pct));
    assert!(!display_range.should_display(&ctx_150_pct));
}

#[test]
fn test_one_of_condition() {
    let display = ParameterDisplay::new().show_when(DisplayRule::when(
        ParameterKey::new("method").unwrap(),
        DisplayCondition::OneOf(vec![
            Value::text("GET"),
            Value::text("POST"),
            Value::text("PUT"),
        ]),
    ));

    let ctx_get =
        DisplayContext::new().with_value(ParameterKey::new("method").unwrap(), Value::text("GET"));
    let ctx_post =
        DisplayContext::new().with_value(ParameterKey::new("method").unwrap(), Value::text("POST"));
    let ctx_delete = DisplayContext::new()
        .with_value(ParameterKey::new("method").unwrap(), Value::text("DELETE"));

    assert!(display.should_display(&ctx_get));
    assert!(display.should_display(&ctx_post));
    assert!(!display.should_display(&ctx_delete));
}

#[test]
fn test_is_empty_is_not_empty_conditions() {
    // IsEmpty
    let display_empty = ParameterDisplay::new().show_when(DisplayRule::when(
        ParameterKey::new("value").unwrap(),
        DisplayCondition::IsEmpty,
    ));

    let ctx_empty =
        DisplayContext::new().with_value(ParameterKey::new("value").unwrap(), Value::text(""));
    let ctx_not_empty =
        DisplayContext::new().with_value(ParameterKey::new("value").unwrap(), Value::text("hello"));

    assert!(display_empty.should_display(&ctx_empty));
    assert!(!display_empty.should_display(&ctx_not_empty));

    // IsNotEmpty
    let display_not_empty = ParameterDisplay::new().show_when(DisplayRule::when(
        ParameterKey::new("value").unwrap(),
        DisplayCondition::IsNotEmpty,
    ));

    assert!(!display_not_empty.should_display(&ctx_empty));
    assert!(display_not_empty.should_display(&ctx_not_empty));
}

#[test]
fn test_hide_when_takes_precedence() {
    let param = TextParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("field")
                .name("Field")
                .description("Test field")
                .build()
                .unwrap(),
        )
        .display(
            ParameterDisplay::new()
                .show_when_true(ParameterKey::new("show_advanced").unwrap())
                .hide_when_true(ParameterKey::new("maintenance_mode").unwrap()),
        )
        .build();

    // Both show and hide conditions are true - hide should win
    let ctx = DisplayContext::new()
        .with_value(
            ParameterKey::new("show_advanced").unwrap(),
            Value::boolean(true),
        )
        .with_value(
            ParameterKey::new("maintenance_mode").unwrap(),
            Value::boolean(true),
        );

    assert!(!param.should_display(&ctx));
}
