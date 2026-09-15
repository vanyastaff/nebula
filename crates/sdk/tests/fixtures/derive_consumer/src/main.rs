use nebula::integration::credential::StaticResolveResult;
use nebula::prelude::*;

#[derive(Debug, Deserialize, Schema)]
#[serde(crate = "nebula::serde")]
struct SchemaPayload {
    #[property(
        display(label = "Name", widget = text),
        input(expressions = forbidden),
        validate(non_empty, length(min = 1, max = 64))
    )]
    name: String,
    #[property(validate(items(min = 1, max = 3), unique))]
    tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Schema)]
#[serde(crate = "nebula::serde")]
struct NestedPayload {
    child: SchemaPayload,
}

#[derive(Debug, Deserialize, Schema, PartialEq)]
#[serde(crate = "nebula::serde")]
struct UnitPayload;

#[derive(Debug, Deserialize, Schema, PartialEq)]
#[serde(crate = "nebula::serde")]
struct EmptyRecord {}

#[derive(Action)]
#[action(key = "contract.scalar", name = "Scalar contract", input = u8, output = ())]
struct ScalarAction;

#[derive(Validator)]
struct ValidatedPayload {
    #[validate(min_length = 1)]
    name: String,
}

#[derive(Action)]
#[action(
    key = "contract.action",
    name = "Contract action",
    version = "2.5.0-rc.7+build.009",
    input = Value,
    output = Value
)]
struct ContractAction;

#[derive(Debug, Plugin)]
#[plugin(key = "contract", name = "Contract plugin")]
struct ContractPlugin;

#[derive(Resource)]
struct ContractResource;

#[derive(Clone, Schema, ResourceConfig)]
#[config(schema = external)]
struct ContractConfig {
    #[property(display(widget = checkbox))]
    enabled: bool,
}

#[derive(Clone, ResourceConfig)]
struct UnitConfig;

#[derive(AuthScheme)]
#[auth_scheme(pattern = NoAuth, family = NoAuthFamily, public)]
struct ContractAuthScheme {}

struct ContractCredential;

#[derive(Debug, Deserialize, Schema)]
#[serde(crate = "nebula::serde")]
struct CredentialProperties {
    #[property(
        display(label = "Token", widget = password),
        input(secret, expressions = forbidden),
        validate(non_empty)
    )]
    token: SecretString,
}

#[credential(key = "contract.credential", name = "Contract credential")]
impl ContractCredential {
    type Properties = CredentialProperties;
    type Scheme = SecretToken;
    type State = SecretToken;

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        properties: &CredentialProperties,
        _context: &CredentialContext,
    ) -> Result<StaticResolveResult<SecretToken>, CredentialError> {
        Ok(StaticResolveResult::Complete(SecretToken::new(
            properties.token.clone(),
        )))
    }
}

fn main() {
    let nested: NestedPayload =
        nebula::serde_json::from_value(nebula::json!({ "child": { "name": "nested value" } }))
            .expect("SDK-renamed serde derives decode nested payloads");
    assert_eq!(nested.child.name, "nested value");
    let schema = schema_of::<NestedPayload>().unwrap();
    assert!(schema.find(&field_key!("child")).is_some());
    let scalar = schema_of::<u8>().unwrap();
    let RootShape::Scalar(domain) = scalar.root_shape() else {
        panic!("a primitive must publish a scalar root");
    };
    assert_eq!(domain.kind(), ScalarKind::Integer);
    let unit = schema_of::<UnitPayload>().unwrap();
    assert_eq!(unit.scalar_schema().unwrap().kind(), ScalarKind::Null);
    let record = schema_of::<EmptyRecord>().unwrap();
    assert!(matches!(record.root_shape(), RootShape::Record(_)));
    assert!(
        record
            .validate(nebula::params! { data; () }.unwrap())
            .is_err()
    );
    let _: ActionMetadataDraft = ScalarAction::metadata();

    let payload = ValidatedPayload {
        name: "valid".to_owned(),
    };
    payload.validate_fields().expect("payload is valid");
    let schema = schema_of::<SchemaPayload>().expect("valid test catalog definition");
    let authored = nebula::params! { data; "name" => "{{ literal }}" }.unwrap();
    assert_eq!(
        authored.get("name").and_then(AuthoredValue::as_str),
        Some("{{ literal }}")
    );
    assert!(schema.find(&field_key!("name")).is_some());
    assert!(
        schema
            .validate(nebula::params! { data; "name" => "" }.unwrap())
            .is_err()
    );
    let tagged = schema
        .validate(nebula::params! { data; "name" => "Example", "tags" => ["a", "b"] }.unwrap())
        .unwrap()
        .resolve_data()
        .unwrap()
        .into_typed::<SchemaPayload>()
        .unwrap();
    assert_eq!(tagged.tags, Some(vec!["a".to_owned(), "b".to_owned()]));
    let duplicates = schema
        .validate(nebula::params! { data; "name" => "Example", "tags" => ["a", "a"] }.unwrap())
        .unwrap_err();
    assert!(duplicates.errors().any(|error| error.code() == "items.unique"));
    let properties_schema = schema_of::<CredentialProperties>().unwrap();
    let secret_input = properties_schema
        .validate(nebula::params! { data; "token" => "sdk-property-secret" }.unwrap())
        .unwrap()
        .resolve_data()
        .unwrap();
    assert!(!format!("{secret_input:?}").contains("sdk-property-secret"));
    let properties: CredentialProperties = secret_input
        .into_typed_exposing_secrets()
        .unwrap();
    assert_eq!(properties.token.expose_secret(), "sdk-property-secret");
    assert!(!format!("{properties:?}").contains("sdk-property-secret"));
    let _: CredentialMetadataDraft = ContractCredential::metadata();
    let _: ActionMetadataDraft = <ContractAction as Action>::metadata();
    let _ = ContractPlugin.manifest();
    let _ = ContractConfig { enabled: true }.fingerprint();
    let config = schema_of::<ContractConfig>().expect("named config declares its fields");
    assert!(matches!(
        config.find(&field_key!("enabled")),
        Some(Field::Boolean(_))
    ));
    assert!(
        config
            .validate(nebula::params! { data; "enabled" => "true" }.unwrap())
            .is_err()
    );
    let unit_config = schema_of::<UnitConfig>().expect("unit resource config has a valid schema");
    assert_eq!(
        unit_config.scalar_schema().unwrap().kind(),
        ScalarKind::Null
    );
    let _ = <ContractAuthScheme as AuthSchemeContract>::pattern();
    let _ = ContractResource;
}
