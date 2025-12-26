//! Integration tests for the display condition system

use nebula_parameter::DisplayableMut;
use nebula_parameter::core::ParameterBase;
use nebula_parameter::prelude::*;
use nebula_value::{Array, Value};

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
        .base(
            ParameterBase::new(
                ParameterMetadata::builder()
                    .key("api_key")
                    .name("API Key")
                    .description("Enter your API key")
                    .build()
                    .unwrap(),
            )
            .with_display(ParameterDisplay::new().show_when_equals(
                ParameterKey::new("auth_type").unwrap(),
                Value::text("api_key"),
            )),
        )
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
        .base(
            ParameterBase::new(
                ParameterMetadata::builder()
                    .key("api_key")
                    .name("API Key")
                    .description("Enter your API key")
                    .build()
                    .unwrap(),
            )
            .with_display(ParameterDisplay::new().show_when_equals(
                ParameterKey::new("auth_type").unwrap(),
                Value::text("api_key"),
            )),
        )
        .build();

    // OAuth client ID (shown when auth_type is "oauth")
    let oauth_client_id_param = TextParameter::builder()
        .base(
            ParameterBase::new(
                ParameterMetadata::builder()
                    .key("oauth_client_id")
                    .name("OAuth Client ID")
                    .description("Enter your OAuth client ID")
                    .build()
                    .unwrap(),
            )
            .with_display(ParameterDisplay::new().show_when_equals(
                ParameterKey::new("auth_type").unwrap(),
                Value::text("oauth"),
            )),
        )
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
        .base(
            ParameterBase::new(
                ParameterMetadata::builder()
                    .key("field")
                    .name("Field")
                    .description("Test field")
                    .build()
                    .unwrap(),
            )
            .with_display(
                ParameterDisplay::new()
                    .show_when_true(ParameterKey::new("show_advanced").unwrap())
                    .hide_when_true(ParameterKey::new("maintenance_mode").unwrap()),
            ),
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

// =============================================================================
// Additional coverage tests
// =============================================================================

#[test]
fn test_textarea_parameter_with_display() {
    let param = TextareaParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("notes")
                .name("Notes")
                .description("Additional notes")
                .build()
                .unwrap(),
        )
        .display(ParameterDisplay::new().show_when_true(ParameterKey::new("show_notes").unwrap()))
        .build();

    let ctx_show = DisplayContext::new().with_value(
        ParameterKey::new("show_notes").unwrap(),
        Value::boolean(true),
    );
    let ctx_hide = DisplayContext::new().with_value(
        ParameterKey::new("show_notes").unwrap(),
        Value::boolean(false),
    );

    assert!(param.should_display(&ctx_show));
    assert!(!param.should_display(&ctx_hide));
}

#[test]
fn test_secret_parameter_with_display() {
    let param = SecretParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("password")
                .name("Password")
                .description("Enter password")
                .build()
                .unwrap(),
        )
        .display(ParameterDisplay::new().show_when_equals(
            ParameterKey::new("auth_method").unwrap(),
            Value::text("password"),
        ))
        .build();

    let ctx_show = DisplayContext::new().with_value(
        ParameterKey::new("auth_method").unwrap(),
        Value::text("password"),
    );
    let ctx_hide = DisplayContext::new().with_value(
        ParameterKey::new("auth_method").unwrap(),
        Value::text("token"),
    );

    assert!(param.should_display(&ctx_show));
    assert!(!param.should_display(&ctx_hide));
}

#[test]
fn test_color_parameter_with_display() {
    let param = ColorParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("theme_color")
                .name("Theme Color")
                .description("Custom theme color")
                .build()
                .unwrap(),
        )
        .display(
            ParameterDisplay::new()
                .show_when_equals(ParameterKey::new("theme").unwrap(), Value::text("custom")),
        )
        .build();

    let ctx_show = DisplayContext::new()
        .with_value(ParameterKey::new("theme").unwrap(), Value::text("custom"));
    let ctx_hide =
        DisplayContext::new().with_value(ParameterKey::new("theme").unwrap(), Value::text("dark"));

    assert!(param.should_display(&ctx_show));
    assert!(!param.should_display(&ctx_hide));
}

#[test]
fn test_code_parameter_with_display() {
    let param = CodeParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("custom_code")
                .name("Custom Code")
                .description("Enter custom code")
                .build()
                .unwrap(),
        )
        .display(
            ParameterDisplay::new()
                .show_when_true(ParameterKey::new("enable_custom_code").unwrap()),
        )
        .build();

    let ctx_show = DisplayContext::new().with_value(
        ParameterKey::new("enable_custom_code").unwrap(),
        Value::boolean(true),
    );

    assert!(param.should_display(&ctx_show));
}

#[test]
fn test_date_parameter_with_display() {
    let param = DateParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("start_date")
                .name("Start Date")
                .description("Select start date")
                .build()
                .unwrap(),
        )
        .display(
            ParameterDisplay::new().show_when_true(ParameterKey::new("use_date_range").unwrap()),
        )
        .build();

    let ctx_show = DisplayContext::new().with_value(
        ParameterKey::new("use_date_range").unwrap(),
        Value::boolean(true),
    );

    assert!(param.should_display(&ctx_show));
}

#[test]
fn test_time_parameter_with_display() {
    let param = TimeParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("start_time")
                .name("Start Time")
                .description("Select start time")
                .build()
                .unwrap(),
        )
        .display(
            ParameterDisplay::new().show_when_true(ParameterKey::new("schedule_enabled").unwrap()),
        )
        .build();

    let ctx_show = DisplayContext::new().with_value(
        ParameterKey::new("schedule_enabled").unwrap(),
        Value::boolean(true),
    );

    assert!(param.should_display(&ctx_show));
}

#[test]
fn test_datetime_parameter_with_display() {
    let param = DateTimeParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("expires_at")
                .name("Expires At")
                .description("Expiration datetime")
                .build()
                .unwrap(),
        )
        .display(
            ParameterDisplay::new().show_when_true(ParameterKey::new("set_expiration").unwrap()),
        )
        .build();

    let ctx_show = DisplayContext::new().with_value(
        ParameterKey::new("set_expiration").unwrap(),
        Value::boolean(true),
    );
    let ctx_hide = DisplayContext::new().with_value(
        ParameterKey::new("set_expiration").unwrap(),
        Value::boolean(false),
    );

    assert!(param.should_display(&ctx_show));
    assert!(!param.should_display(&ctx_hide));
}

#[test]
fn test_radio_parameter_with_display() {
    let param = RadioParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("priority")
                .name("Priority")
                .description("Select priority")
                .build()
                .unwrap(),
        )
        .options(vec![])
        .display(
            ParameterDisplay::new().show_when_true(ParameterKey::new("advanced_settings").unwrap()),
        )
        .build();

    let ctx_show = DisplayContext::new().with_value(
        ParameterKey::new("advanced_settings").unwrap(),
        Value::boolean(true),
    );

    assert!(param.should_display(&ctx_show));
}

#[test]
fn test_multi_select_parameter_with_display() {
    let param = MultiSelectParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("tags")
                .name("Tags")
                .description("Select tags")
                .build()
                .unwrap(),
        )
        .options(vec![])
        .display(
            ParameterDisplay::new().show_when_true(ParameterKey::new("enable_tagging").unwrap()),
        )
        .build();

    let ctx_show = DisplayContext::new().with_value(
        ParameterKey::new("enable_tagging").unwrap(),
        Value::boolean(true),
    );

    assert!(param.should_display(&ctx_show));
}

#[test]
fn test_file_parameter_with_display() {
    let param = FileParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("upload_file")
                .name("Upload File")
                .description("Select file to upload")
                .build()
                .unwrap(),
        )
        .display(ParameterDisplay::new().show_when_equals(
            ParameterKey::new("input_type").unwrap(),
            Value::text("file"),
        ))
        .build();

    let ctx_show = DisplayContext::new().with_value(
        ParameterKey::new("input_type").unwrap(),
        Value::text("file"),
    );
    let ctx_hide = DisplayContext::new().with_value(
        ParameterKey::new("input_type").unwrap(),
        Value::text("text"),
    );

    assert!(param.should_display(&ctx_show));
    assert!(!param.should_display(&ctx_hide));
}

#[test]
fn test_list_parameter_with_display() {
    let param = ListParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("items")
                .name("Items")
                .description("Enter items")
                .build()
                .unwrap(),
        )
        .children(vec![])
        .display(ParameterDisplay::new().show_when_true(ParameterKey::new("enable_list").unwrap()))
        .build();

    let ctx_show = DisplayContext::new().with_value(
        ParameterKey::new("enable_list").unwrap(),
        Value::boolean(true),
    );

    assert!(param.should_display(&ctx_show));
}

#[test]
fn test_object_parameter_with_display() {
    let mut param = ObjectParameter::new("config", "Configuration", "Custom configuration object")
        .expect("should create parameter");
    param.display = Some(
        ParameterDisplay::new().show_when_true(ParameterKey::new("use_custom_config").unwrap()),
    );

    let ctx_show = DisplayContext::new().with_value(
        ParameterKey::new("use_custom_config").unwrap(),
        Value::boolean(true),
    );

    assert!(param.should_display(&ctx_show));
}

#[test]
fn test_group_parameter_with_display() {
    let param = GroupParameter::builder()
        .metadata(
            ParameterMetadata::builder()
                .key("advanced_group")
                .name("Advanced Settings")
                .description("Advanced configuration group")
                .build()
                .unwrap(),
        )
        .fields(vec![])
        .display(
            ParameterDisplay::new().show_when_true(ParameterKey::new("show_advanced").unwrap()),
        )
        .build();

    let ctx_show = DisplayContext::new().with_value(
        ParameterKey::new("show_advanced").unwrap(),
        Value::boolean(true),
    );
    let ctx_hide = DisplayContext::new().with_value(
        ParameterKey::new("show_advanced").unwrap(),
        Value::boolean(false),
    );

    assert!(param.should_display(&ctx_show));
    assert!(!param.should_display(&ctx_hide));
}

#[test]
fn test_resource_parameter_with_display() {
    let metadata = ParameterMetadata::builder()
        .key("database")
        .name("Database")
        .description("Select database resource")
        .build()
        .unwrap();

    let mut param = ResourceParameter::new(metadata);
    param.display = Some(ParameterDisplay::new().show_when_equals(
        ParameterKey::new("storage_type").unwrap(),
        Value::text("database"),
    ));

    let ctx_show = DisplayContext::new().with_value(
        ParameterKey::new("storage_type").unwrap(),
        Value::text("database"),
    );

    assert!(param.should_display(&ctx_show));
}

// =============================================================================
// Edge cases and advanced scenarios
// =============================================================================

#[test]
fn test_display_with_null_value() {
    let display = ParameterDisplay::new().show_when(DisplayRule::when(
        ParameterKey::new("field").unwrap(),
        DisplayCondition::IsNull,
    ));

    let ctx_null =
        DisplayContext::new().with_value(ParameterKey::new("field").unwrap(), Value::Null);
    let ctx_set =
        DisplayContext::new().with_value(ParameterKey::new("field").unwrap(), Value::text("value"));

    assert!(display.should_display(&ctx_null));
    assert!(!display.should_display(&ctx_set));
}

#[test]
fn test_display_with_is_set() {
    let display = ParameterDisplay::new().show_when(DisplayRule::when(
        ParameterKey::new("field").unwrap(),
        DisplayCondition::IsSet,
    ));

    let ctx_set =
        DisplayContext::new().with_value(ParameterKey::new("field").unwrap(), Value::text("value"));
    let ctx_null =
        DisplayContext::new().with_value(ParameterKey::new("field").unwrap(), Value::Null);
    let ctx_missing = DisplayContext::new();

    assert!(display.should_display(&ctx_set));
    assert!(!display.should_display(&ctx_null));
    assert!(!display.should_display(&ctx_missing)); // Missing field = not set
}

#[test]
fn test_display_with_is_false() {
    let display = ParameterDisplay::new().show_when(DisplayRule::when(
        ParameterKey::new("disabled").unwrap(),
        DisplayCondition::IsFalse,
    ));

    let ctx_false = DisplayContext::new().with_value(
        ParameterKey::new("disabled").unwrap(),
        Value::boolean(false),
    );
    let ctx_true = DisplayContext::new()
        .with_value(ParameterKey::new("disabled").unwrap(), Value::boolean(true));

    assert!(display.should_display(&ctx_false));
    assert!(!display.should_display(&ctx_true));
}

#[test]
fn test_display_with_not_equals() {
    let display = ParameterDisplay::new().show_when(DisplayRule::when(
        ParameterKey::new("status").unwrap(),
        DisplayCondition::NotEquals(Value::text("disabled")),
    ));

    let ctx_active = DisplayContext::new()
        .with_value(ParameterKey::new("status").unwrap(), Value::text("active"));
    let ctx_disabled = DisplayContext::new().with_value(
        ParameterKey::new("status").unwrap(),
        Value::text("disabled"),
    );

    assert!(display.should_display(&ctx_active));
    assert!(!display.should_display(&ctx_disabled));
}

#[test]
fn test_display_empty_ruleset() {
    let display = ParameterDisplay::new();

    // Empty display should always show
    let ctx = DisplayContext::new();
    assert!(display.should_display(&ctx));
    assert!(display.is_empty());
}

#[test]
fn test_display_multiple_show_conditions_combined() {
    // Multiple show_when calls should be AND-ed together
    let display = ParameterDisplay::new()
        .show_when(DisplayRule::when(
            ParameterKey::new("a").unwrap(),
            DisplayCondition::IsTrue,
        ))
        .show_when(DisplayRule::when(
            ParameterKey::new("b").unwrap(),
            DisplayCondition::IsTrue,
        ));

    let ctx_both = DisplayContext::new()
        .with_value(ParameterKey::new("a").unwrap(), Value::boolean(true))
        .with_value(ParameterKey::new("b").unwrap(), Value::boolean(true));

    let ctx_only_a = DisplayContext::new()
        .with_value(ParameterKey::new("a").unwrap(), Value::boolean(true))
        .with_value(ParameterKey::new("b").unwrap(), Value::boolean(false));

    assert!(display.should_display(&ctx_both));
    assert!(!display.should_display(&ctx_only_a));
}

#[test]
fn test_display_multiple_hide_conditions_combined() {
    // Multiple hide_when calls should be OR-ed together
    let display = ParameterDisplay::new()
        .hide_when(DisplayRule::when(
            ParameterKey::new("maintenance").unwrap(),
            DisplayCondition::IsTrue,
        ))
        .hide_when(DisplayRule::when(
            ParameterKey::new("deprecated").unwrap(),
            DisplayCondition::IsTrue,
        ));

    let ctx_maintenance = DisplayContext::new()
        .with_value(
            ParameterKey::new("maintenance").unwrap(),
            Value::boolean(true),
        )
        .with_value(
            ParameterKey::new("deprecated").unwrap(),
            Value::boolean(false),
        );

    let ctx_deprecated = DisplayContext::new()
        .with_value(
            ParameterKey::new("maintenance").unwrap(),
            Value::boolean(false),
        )
        .with_value(
            ParameterKey::new("deprecated").unwrap(),
            Value::boolean(true),
        );

    let ctx_neither = DisplayContext::new()
        .with_value(
            ParameterKey::new("maintenance").unwrap(),
            Value::boolean(false),
        )
        .with_value(
            ParameterKey::new("deprecated").unwrap(),
            Value::boolean(false),
        );

    assert!(!display.should_display(&ctx_maintenance));
    assert!(!display.should_display(&ctx_deprecated));
    assert!(display.should_display(&ctx_neither));
}

#[test]
fn test_display_with_float_values() {
    let display = ParameterDisplay::new().show_when(DisplayRule::when(
        ParameterKey::new("ratio").unwrap(),
        DisplayCondition::InRange { min: 0.0, max: 1.0 },
    ));

    let ctx_valid =
        DisplayContext::new().with_value(ParameterKey::new("ratio").unwrap(), Value::float(0.5));
    let ctx_invalid =
        DisplayContext::new().with_value(ParameterKey::new("ratio").unwrap(), Value::float(1.5));

    assert!(display.should_display(&ctx_valid));
    assert!(!display.should_display(&ctx_invalid));
}

#[test]
fn test_display_with_empty_array() {
    let display = ParameterDisplay::new().show_when(DisplayRule::when(
        ParameterKey::new("items").unwrap(),
        DisplayCondition::IsEmpty,
    ));

    let ctx_empty =
        DisplayContext::new().with_value(ParameterKey::new("items").unwrap(), Value::array_empty());
    let ctx_with_items = DisplayContext::new().with_value(
        ParameterKey::new("items").unwrap(),
        Value::Array(Array::from_vec(vec![Value::integer(1).into()])),
    );

    assert!(display.should_display(&ctx_empty));
    assert!(!display.should_display(&ctx_with_items));
}

#[test]
fn test_display_with_empty_object() {
    let display = ParameterDisplay::new().show_when(DisplayRule::when(
        ParameterKey::new("config").unwrap(),
        DisplayCondition::IsEmpty,
    ));

    let ctx_empty = DisplayContext::new()
        .with_value(ParameterKey::new("config").unwrap(), Value::object_empty());

    assert!(display.should_display(&ctx_empty));
}

#[test]
fn test_deeply_nested_ruleset() {
    // ((A AND B) OR (C AND D)) AND NOT E
    let ruleset = DisplayRuleSet::all([
        DisplayRuleSet::any([
            DisplayRuleSet::all([
                DisplayRule::when(ParameterKey::new("a").unwrap(), DisplayCondition::IsTrue),
                DisplayRule::when(ParameterKey::new("b").unwrap(), DisplayCondition::IsTrue),
            ]),
            DisplayRuleSet::all([
                DisplayRule::when(ParameterKey::new("c").unwrap(), DisplayCondition::IsTrue),
                DisplayRule::when(ParameterKey::new("d").unwrap(), DisplayCondition::IsTrue),
            ]),
        ]),
        DisplayRuleSet::not(DisplayRule::when(
            ParameterKey::new("e").unwrap(),
            DisplayCondition::IsTrue,
        )),
    ]);

    let display = ParameterDisplay::new().show_when(ruleset);

    // A=true, B=true, E=false -> should show
    let ctx1 = DisplayContext::new()
        .with_value(ParameterKey::new("a").unwrap(), Value::boolean(true))
        .with_value(ParameterKey::new("b").unwrap(), Value::boolean(true))
        .with_value(ParameterKey::new("e").unwrap(), Value::boolean(false));

    // C=true, D=true, E=false -> should show
    let ctx2 = DisplayContext::new()
        .with_value(ParameterKey::new("c").unwrap(), Value::boolean(true))
        .with_value(ParameterKey::new("d").unwrap(), Value::boolean(true))
        .with_value(ParameterKey::new("e").unwrap(), Value::boolean(false));

    // A=true, B=true, E=true -> should NOT show (E blocks it)
    let ctx3 = DisplayContext::new()
        .with_value(ParameterKey::new("a").unwrap(), Value::boolean(true))
        .with_value(ParameterKey::new("b").unwrap(), Value::boolean(true))
        .with_value(ParameterKey::new("e").unwrap(), Value::boolean(true));

    assert!(display.should_display(&ctx1));
    assert!(display.should_display(&ctx2));
    assert!(!display.should_display(&ctx3));
}

#[test]
fn test_display_context_insert_and_contains() {
    let mut ctx = DisplayContext::new();

    assert!(!ctx.contains("key"));

    ctx.insert(ParameterKey::new("key").unwrap(), Value::text("value"));

    assert!(ctx.contains("key"));
    assert_eq!(ctx.get("key"), Some(&Value::text("value")));
}

#[test]
fn test_display_context_values() {
    let ctx = DisplayContext::new()
        .with_value(ParameterKey::new("a").unwrap(), Value::integer(1))
        .with_value(ParameterKey::new("b").unwrap(), Value::integer(2));

    let values = ctx.values();
    assert_eq!(values.len(), 2);
}

#[test]
fn test_displayable_trait_methods() {
    let mut param = TextParameter::builder()
        .base(ParameterBase::new(
            ParameterMetadata::builder()
                .key("test")
                .name("Test")
                .description("Test parameter")
                .build()
                .unwrap(),
        ))
        .build();

    // Initially no conditions
    assert!(!param.has_conditions());
    assert!(param.dependencies().is_empty());

    // Add a show condition via set_display
    param.set_display(Some(ParameterDisplay::new().show_when(DisplayRule::when(
        ParameterKey::new("trigger").unwrap(),
        DisplayCondition::IsTrue,
    ))));

    assert!(param.has_conditions());
    assert!(
        param
            .dependencies()
            .contains(&ParameterKey::new("trigger").unwrap())
    );

    // Clear conditions
    param.clear_conditions();
    assert!(!param.has_conditions());
}

#[test]
fn test_displayable_mut_hide_condition() {
    let mut param = TextParameter::builder()
        .base(ParameterBase::new(
            ParameterMetadata::builder()
                .key("test")
                .name("Test")
                .description("Test parameter")
                .build()
                .unwrap(),
        ))
        .build();

    // Add hide condition via set_display
    param.set_display(Some(
        ParameterDisplay::new().hide_when_true(ParameterKey::new("hidden").unwrap()),
    ));

    let ctx_hidden = DisplayContext::new()
        .with_value(ParameterKey::new("hidden").unwrap(), Value::boolean(true));
    let ctx_visible = DisplayContext::new()
        .with_value(ParameterKey::new("hidden").unwrap(), Value::boolean(false));

    assert!(!param.should_display(&ctx_hidden));
    assert!(param.should_display(&ctx_visible));
}

#[test]
fn test_display_rule_dependency() {
    let rule = DisplayRule::when(
        ParameterKey::new("my_field").unwrap(),
        DisplayCondition::IsTrue,
    );

    assert_eq!(rule.dependency(), &ParameterKey::new("my_field").unwrap());
}

#[test]
fn test_display_ruleset_single() {
    let rule = DisplayRule::when(
        ParameterKey::new("field").unwrap(),
        DisplayCondition::IsTrue,
    );
    let ruleset = DisplayRuleSet::single(rule);

    let ctx =
        DisplayContext::new().with_value(ParameterKey::new("field").unwrap(), Value::boolean(true));

    assert!(ruleset.evaluate(&ctx));
}

#[test]
fn test_display_serialization_complex() {
    let display = ParameterDisplay::new()
        .show_when(DisplayRuleSet::all([
            DisplayRule::when(
                ParameterKey::new("enabled").unwrap(),
                DisplayCondition::IsTrue,
            ),
            DisplayRule::when(
                ParameterKey::new("level").unwrap(),
                DisplayCondition::GreaterThan(5.0),
            ),
        ]))
        .hide_when(DisplayRule::when(
            ParameterKey::new("maintenance").unwrap(),
            DisplayCondition::IsTrue,
        ));

    let json = serde_json::to_string(&display).unwrap();
    let restored: ParameterDisplay = serde_json::from_str(&json).unwrap();

    // Test that restored display behaves the same
    let ctx = DisplayContext::new()
        .with_value(ParameterKey::new("enabled").unwrap(), Value::boolean(true))
        .with_value(ParameterKey::new("level").unwrap(), Value::integer(10))
        .with_value(
            ParameterKey::new("maintenance").unwrap(),
            Value::boolean(false),
        );

    assert_eq!(display.should_display(&ctx), restored.should_display(&ctx));
}

#[test]
fn test_one_of_with_integers() {
    let display = ParameterDisplay::new().show_when(DisplayRule::when(
        ParameterKey::new("status_code").unwrap(),
        DisplayCondition::OneOf(vec![
            Value::integer(200),
            Value::integer(201),
            Value::integer(204),
        ]),
    ));

    let ctx_200 = DisplayContext::new().with_value(
        ParameterKey::new("status_code").unwrap(),
        Value::integer(200),
    );
    let ctx_404 = DisplayContext::new().with_value(
        ParameterKey::new("status_code").unwrap(),
        Value::integer(404),
    );

    assert!(display.should_display(&ctx_200));
    assert!(!display.should_display(&ctx_404));
}

#[test]
fn test_condition_with_wrong_type() {
    // GreaterThan with string value should return false (not panic)
    let display = ParameterDisplay::new().show_when(DisplayRule::when(
        ParameterKey::new("value").unwrap(),
        DisplayCondition::GreaterThan(10.0),
    ));

    let ctx_string = DisplayContext::new().with_value(
        ParameterKey::new("value").unwrap(),
        Value::text("not a number"),
    );

    // Should not panic, just return false
    assert!(!display.should_display(&ctx_string));
}

#[test]
fn test_string_condition_with_wrong_type() {
    // Contains with integer value should return false (not panic)
    let display = ParameterDisplay::new().show_when(DisplayRule::when(
        ParameterKey::new("value").unwrap(),
        DisplayCondition::Contains("test".to_string()),
    ));

    let ctx_int =
        DisplayContext::new().with_value(ParameterKey::new("value").unwrap(), Value::integer(42));

    // Should not panic, just return false
    assert!(!display.should_display(&ctx_int));
}
