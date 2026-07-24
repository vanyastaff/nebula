use nebula_sdk::prelude::*;

#[derive(schema_leaf::Schema)]
struct SchemaPayload {
    name: String,
}

#[derive(validator_leaf::Validator)]
struct ValidatedPayload {
    #[validate(min_length = 1)]
    name: String,
}

#[derive(action_leaf::Action)]
#[action(
    key = "contract.renamed_action",
    name = "Renamed contract action",
    input = Value,
    output = Value
)]
struct ContractAction;

#[derive(Debug, plugin_leaf::Plugin)]
#[plugin(key = "renamed_contract", name = "Renamed contract plugin")]
struct ContractPlugin;

#[derive(resource_leaf::Resource)]
struct ContractResource;

#[derive(Clone, resource_leaf::ResourceConfig)]
struct ContractConfig {
    enabled: bool,
}

#[derive(credential_leaf::AuthScheme)]
#[auth_scheme(pattern = NoAuth, family = NoAuthFamily, public)]
struct ContractAuthScheme {}

fn main() {
    ValidatedPayload {
        name: "valid".to_owned(),
    }
    .validate_fields()
    .expect("payload is valid");
    let _ = <SchemaPayload as schema_leaf::HasSchema>::schema();
    let _ = <ContractAction as Action>::metadata();
    let _ = ContractPlugin.manifest();
    let _ = ContractConfig { enabled: true }.fingerprint();
    let _ = <ContractAuthScheme as AuthSchemeContract>::pattern();
    let _ = ContractResource;
}
