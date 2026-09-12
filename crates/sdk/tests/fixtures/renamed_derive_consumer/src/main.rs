use nebula_sdk::integration::credential::StaticResolveResult;
use nebula_sdk::prelude::*;

#[derive(Debug, Deserialize, schema_leaf::Schema)]
#[serde(crate = "nebula_sdk::serde")]
struct SchemaPayload {
    name: String,
}

#[derive(Debug, Deserialize, schema_leaf::Schema)]
#[serde(crate = "nebula_sdk::serde")]
struct NestedPayload {
    child: SchemaPayload,
}

#[derive(Debug, Deserialize, schema_leaf::Schema, PartialEq)]
#[serde(crate = "nebula_sdk::serde")]
struct UnitPayload;

#[derive(Debug, Deserialize, schema_leaf::Schema, PartialEq)]
#[serde(crate = "nebula_sdk::serde")]
struct EmptyRecord {}

#[derive(action_leaf::Action)]
#[action(key = "contract.scalar", name = "Scalar contract", input = u8, output = ())]
struct ScalarAction;

#[derive(validator_leaf::Validator)]
struct ValidatedPayload {
    #[validate(min_length = 1)]
    name: String,
}

#[derive(action_leaf::Action)]
#[action(
    key = "contract.renamed_action",
    name = "Renamed contract action",
    version = "2.5.0-rc.7+build.009",
    input = Value,
    output = Value
)]
struct ContractAction;

#[derive(Debug, plugin_leaf::Plugin)]
#[plugin(key = "renamed_contract", name = "Renamed contract plugin")]
struct ContractPlugin;

#[derive(resource_leaf::Resource)]
struct ContractResource;

#[derive(Clone, schema_leaf::Schema, resource_leaf::ResourceConfig)]
#[config(schema = external)]
struct ContractConfig {
    enabled: bool,
}

#[derive(credential_leaf::AuthScheme)]
#[auth_scheme(pattern = NoAuth, family = NoAuthFamily, public)]
struct ContractAuthScheme {}

struct ContractCredential;

#[derive(Debug, Deserialize, schema_leaf::Schema)]
#[serde(crate = "nebula_sdk::serde")]
struct CredentialProperties {
    token: String,
}

#[credential_leaf::credential(
    key = "contract.renamed_credential",
    name = "Renamed contract credential"
)]
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
            SecretString::new(properties.token.clone()),
        )))
    }
}

fn main() {
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
            .validate(nebula_sdk::params! { data; () }.unwrap())
            .is_err()
    );
    let _: ActionMetadataDraft = ScalarAction::metadata();

    ValidatedPayload {
        name: "valid".to_owned(),
    }
    .validate_fields()
    .expect("payload is valid");
    let schema = schema_of::<SchemaPayload>().expect("valid test catalog definition");
    let authored = nebula_sdk::params! { data; "name" => "{{ literal }}" }.unwrap();
    assert_eq!(
        authored.get("name").and_then(AuthoredValue::as_str),
        Some("{{ literal }}")
    );
    assert!(schema.find(&field_key!("name")).is_some());
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
            .validate(nebula_sdk::params! { data; "enabled" => "true" }.unwrap())
            .is_err()
    );
    let _ = <ContractAuthScheme as AuthSchemeContract>::pattern();
    let _ = ContractResource;
}
