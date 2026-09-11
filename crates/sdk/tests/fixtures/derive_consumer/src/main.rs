use nebula::integration::credential::StaticResolveResult;
use nebula::prelude::*;

#[derive(Debug, Deserialize, Schema)]
#[serde(crate = "nebula::serde")]
struct SchemaPayload {
    name: String,
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
    token: String,
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
    assert_eq!(authored.get("name").and_then(AuthoredValue::as_str), Some("{{ literal }}"));
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
