use nebula_parameter::collection::ParameterCollection;
use nebula_parameter::def::ParameterDef;
use nebula_parameter::kind::ParameterKind;
use nebula_parameter::option::SelectOption;
use nebula_parameter::types::*;
use nebula_parameter::validation::ValidationRule;
use serde_json::json;

// ---------------------------------------------------------------------------
// 1. Add and retrieve by key
// ---------------------------------------------------------------------------

#[test]
fn add_and_get_by_key() {
    let mut col = ParameterCollection::new();
    col.add(ParameterDef::Text(TextParameter::new("host", "Hostname")));
    col.add(ParameterDef::Number(NumberParameter::new("port", "Port")));

    let host = col.get_by_key("host").expect("host should exist");
    assert_eq!(host.key(), "host");
    assert_eq!(host.kind(), ParameterKind::Text);

    let port = col.get_by_key("port").expect("port should exist");
    assert_eq!(port.key(), "port");
    assert_eq!(port.kind(), ParameterKind::Number);

    assert!(col.get_by_key("missing").is_none());
}

#[test]
fn add_returns_self_for_chaining() {
    let mut col = ParameterCollection::new();
    col.add(ParameterDef::Text(TextParameter::new("a", "A")))
        .add(ParameterDef::Text(TextParameter::new("b", "B")));
    assert_eq!(col.len(), 2);
}

#[test]
fn get_by_index() {
    let col = ParameterCollection::new()
        .with(ParameterDef::Text(TextParameter::new("first", "First")))
        .with(ParameterDef::Number(NumberParameter::new(
            "second", "Second",
        )));

    assert_eq!(col.get(0).unwrap().key(), "first");
    assert_eq!(col.get(1).unwrap().key(), "second");
    assert!(col.get(2).is_none());
}

// ---------------------------------------------------------------------------
// 2. Remove and contains
// ---------------------------------------------------------------------------

#[test]
fn remove_existing_key() {
    let mut col = ParameterCollection::new()
        .with(ParameterDef::Text(TextParameter::new("a", "A")))
        .with(ParameterDef::Number(NumberParameter::new("b", "B")))
        .with(ParameterDef::Checkbox(CheckboxParameter::new("c", "C")));

    assert!(col.contains("b"));
    let removed = col.remove("b").expect("should remove 'b'");
    assert_eq!(removed.key(), "b");
    assert!(!col.contains("b"));
    assert_eq!(col.len(), 2);
}

#[test]
fn remove_missing_key_returns_none() {
    let mut col = ParameterCollection::new().with(ParameterDef::Text(TextParameter::new("a", "A")));
    assert!(col.remove("nonexistent").is_none());
    assert_eq!(col.len(), 1);
}

#[test]
fn contains_returns_correct_values() {
    let col = ParameterCollection::new()
        .with(ParameterDef::Text(TextParameter::new("host", "Host")))
        .with(ParameterDef::Secret(SecretParameter::new("token", "Token")));

    assert!(col.contains("host"));
    assert!(col.contains("token"));
    assert!(!col.contains("port"));
    assert!(!col.contains(""));
}

// ---------------------------------------------------------------------------
// 3. Iteration order is preserved
// ---------------------------------------------------------------------------

#[test]
fn iteration_preserves_insertion_order() {
    let keys_in_order = ["alpha", "beta", "gamma", "delta", "epsilon"];

    let col: ParameterCollection = keys_in_order
        .iter()
        .map(|k| ParameterDef::Text(TextParameter::new(*k, *k)))
        .collect();

    let iterated_keys: Vec<&str> = col.iter().map(|p| p.key()).collect();
    assert_eq!(iterated_keys, keys_in_order);
}

#[test]
fn keys_iterator_preserves_insertion_order() {
    let col = ParameterCollection::new()
        .with(ParameterDef::Text(TextParameter::new("z", "Z")))
        .with(ParameterDef::Text(TextParameter::new("a", "A")))
        .with(ParameterDef::Text(TextParameter::new("m", "M")));

    let keys: Vec<&str> = col.keys().collect();
    assert_eq!(keys, vec!["z", "a", "m"]);
}

#[test]
fn into_iter_preserves_order() {
    let col = ParameterCollection::new()
        .with(ParameterDef::Text(TextParameter::new("x", "X")))
        .with(ParameterDef::Text(TextParameter::new("y", "Y")));

    let keys: Vec<String> = col.into_iter().map(|p| p.key().to_owned()).collect();
    assert_eq!(keys, vec!["x", "y"]);
}

#[test]
fn ref_into_iter_preserves_order() {
    let col = ParameterCollection::new()
        .with(ParameterDef::Text(TextParameter::new("p", "P")))
        .with(ParameterDef::Text(TextParameter::new("q", "Q")));

    let keys: Vec<&str> = (&col).into_iter().map(|p| p.key()).collect();
    assert_eq!(keys, vec!["p", "q"]);
}

// ---------------------------------------------------------------------------
// 4. FromIterator
// ---------------------------------------------------------------------------

#[test]
fn from_iterator_collects_correctly() {
    let defs = vec![
        ParameterDef::Text(TextParameter::new("a", "A")),
        ParameterDef::Number(NumberParameter::new("b", "B")),
        ParameterDef::Checkbox(CheckboxParameter::new("c", "C")),
    ];

    let col: ParameterCollection = defs.into_iter().collect();
    assert_eq!(col.len(), 3);
    assert_eq!(col.get(0).unwrap().key(), "a");
    assert_eq!(col.get(1).unwrap().key(), "b");
    assert_eq!(col.get(2).unwrap().key(), "c");
}

#[test]
fn from_empty_iterator() {
    let col: ParameterCollection = Vec::<ParameterDef>::new().into_iter().collect();
    assert!(col.is_empty());
}

// ---------------------------------------------------------------------------
// 5. Realistic credential schema: OAuth2
// ---------------------------------------------------------------------------

#[test]
fn oauth2_credential_schema() {
    let mut client_id = TextParameter::new("client_id", "Client ID");
    client_id.metadata.required = true;
    client_id.metadata.description = Some("OAuth2 client identifier".into());
    client_id.metadata.placeholder = Some("Enter your client ID".into());
    client_id.validation = vec![
        ValidationRule::min_length(10),
        ValidationRule::max_length(128),
    ];

    let mut client_secret = SecretParameter::new("client_secret", "Client Secret");
    client_secret.metadata.required = true;
    client_secret.metadata.description = Some("OAuth2 client secret".into());
    client_secret.validation = vec![ValidationRule::min_length(20)];

    let mut scopes = MultiSelectParameter::new("scopes", "Scopes");
    scopes.metadata.description = Some("Requested OAuth2 scopes".into());
    scopes.options = vec![
        SelectOption::new("read", "Read", json!("read")),
        SelectOption::new("write", "Write", json!("write")),
        SelectOption::new("admin", "Admin", json!("admin")),
    ];
    scopes.default = Some(vec![json!("read")]);

    let mut auth_url = TextParameter::new("auth_url", "Authorization URL");
    auth_url.metadata.required = true;
    auth_url.default = Some("https://github.com/login/oauth/authorize".into());
    auth_url.validation = vec![ValidationRule::pattern(r"^https://")];

    let mut token_url = TextParameter::new("token_url", "Token URL");
    token_url.metadata.required = true;
    token_url.default = Some("https://github.com/login/oauth/access_token".into());

    let redirect_notice = NoticeParameter::new(
        "redirect_info",
        "Redirect URI",
        NoticeType::Info,
        "Set your redirect URI to: https://nebula.local/callback",
    );

    let schema = ParameterCollection::new()
        .with(ParameterDef::Text(client_id))
        .with(ParameterDef::Secret(client_secret))
        .with(ParameterDef::MultiSelect(scopes))
        .with(ParameterDef::Text(auth_url))
        .with(ParameterDef::Text(token_url))
        .with(ParameterDef::Notice(redirect_notice));

    // Verify structure.
    assert_eq!(schema.len(), 6);

    // Required fields.
    assert!(schema.get_by_key("client_id").unwrap().is_required());
    assert!(schema.get_by_key("client_secret").unwrap().is_required());
    assert!(!schema.get_by_key("scopes").unwrap().is_required());

    // Sensitive fields.
    assert!(schema.get_by_key("client_secret").unwrap().is_sensitive());
    assert!(!schema.get_by_key("client_id").unwrap().is_sensitive());

    // Serde round-trip of the full schema.
    let json_str = serde_json::to_string_pretty(&schema).unwrap();
    let restored: ParameterCollection = serde_json::from_str(&json_str).unwrap();

    assert_eq!(restored.len(), 6);
    assert_eq!(
        restored.get_by_key("client_id").unwrap().kind(),
        ParameterKind::Text
    );
    assert_eq!(
        restored.get_by_key("client_secret").unwrap().kind(),
        ParameterKind::Secret
    );
    assert_eq!(
        restored.get_by_key("scopes").unwrap().kind(),
        ParameterKind::MultiSelect
    );
    assert_eq!(
        restored.get_by_key("redirect_info").unwrap().kind(),
        ParameterKind::Notice
    );
}

#[test]
fn database_credential_schema() {
    let mut host = TextParameter::new("host", "Host");
    host.metadata.required = true;
    host.default = Some("localhost".into());

    let mut port = NumberParameter::new("port", "Port");
    port.metadata.required = true;
    port.default = Some(5432.0);
    port.validation = ValidationRule::range(1.0, 65535.0);

    let mut username = TextParameter::new("username", "Username");
    username.metadata.required = true;

    let mut password = SecretParameter::new("password", "Password");
    password.metadata.required = true;

    let mut database = TextParameter::new("database", "Database Name");
    database.metadata.required = true;

    let mut ssl = CheckboxParameter::new("ssl", "Use SSL");
    ssl.default = Some(true);

    let schema = ParameterCollection::new()
        .with(ParameterDef::Text(host))
        .with(ParameterDef::Number(port))
        .with(ParameterDef::Text(username))
        .with(ParameterDef::Secret(password))
        .with(ParameterDef::Text(database))
        .with(ParameterDef::Checkbox(ssl));

    assert_eq!(schema.len(), 6);

    // All except ssl are required.
    let required_keys = ["host", "port", "username", "password", "database"];
    for key in &required_keys {
        assert!(
            schema.get_by_key(key).unwrap().is_required(),
            "{} should be required",
            key
        );
    }
    assert!(!schema.get_by_key("ssl").unwrap().is_required());
}
