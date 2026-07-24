use nebula::prelude::*;

#[derive(Schema)]
struct SchemaPayload {
    name: String,
}

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

#[derive(Clone, ResourceConfig)]
struct ContractConfig {
    enabled: bool,
}

#[derive(AuthScheme)]
#[auth_scheme(pattern = NoAuth, family = NoAuthFamily, public)]
struct ContractAuthScheme {}

fn main() {
    let payload = ValidatedPayload {
        name: "valid".to_owned(),
    };
    payload.validate_fields().expect("payload is valid");
    let _ = <SchemaPayload as nebula::__private::schema::HasSchema>::schema();
    let _ = <ContractAction as Action>::metadata();
    let _ = ContractPlugin.manifest();
    let _ = ContractConfig { enabled: true }.fingerprint();
    let _ = <ContractAuthScheme as AuthSchemeContract>::pattern();
    let _ = ContractResource;
}
