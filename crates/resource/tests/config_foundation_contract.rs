//! Resource config admission must retain schema preparation and reject secrets.

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    marker::PhantomData,
};

use nebula_core::{DeclaresDependencies, Dependencies, ResourceKey, resource_key};
use nebula_resource::{
    Error, HasCredentialSlots, KindActivator, Manager, Provider, RegisterRequest, Resident,
    ResourceConfig, ResourceConfigInput, ResourceContext, ResourceFactory,
};
use nebula_schema::{
    AuthoredValue, Expression, Field, HasSchema, Schema, Transformer, ValidSchema,
    ValidationReport, field_key,
};
use serde::Deserialize;
use serde_json::json;

struct ConfigProbe<C>(PhantomData<C>);

fn factory<C>() -> impl ResourceFactory
where
    C: ResourceConfig + serde::de::DeserializeOwned,
{
    KindActivator::<ConfigProbe<C>, _, _>::new(
        || ConfigProbe(PhantomData),
        || Resident::new(nebula_resource::ResidentConfig::default()),
    )
}

fn validate<C>(config: serde_json::Value) -> Result<(), Error>
where
    C: ResourceConfig + serde::de::DeserializeOwned,
{
    factory::<C>().validate(config)
}

#[test]
fn unit_config_accepts_serde_unit_wire_null() {
    validate::<()>(json!(null)).expect("unit null validates");
}

#[test]
fn unit_config_rejects_supplied_objects_without_discarding_them() {
    for wire in [json!({}), json!({"token": "PRIVATE_UNIT_CONFIG_CANARY"})] {
        let error = validate::<()>(wire).unwrap_err();
        std::assert_matches!(error.kind(), nebula_resource::ErrorKind::Permanent);
        assert!(!format!("{error:?} {error}").contains("PRIVATE_UNIT_CONFIG_CANARY"));
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, serde::Serialize, ResourceConfig)]
#[config(schema = external)]
struct StringConfig(String);

impl HasSchema for StringConfig {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        nebula_schema::schema_of::<String>()
    }
}

#[tokio::test]
async fn scalar_config_preserves_serde_wire_value() {
    let config = StringConfig("https://example.test".to_owned());
    let wire = serde_json::to_value(&config).unwrap();
    assert_eq!(wire, json!("https://example.test"));
    let manager = Manager::new();
    register_config::<StringConfig>(&manager, ResourceConfigInput::data(wire))
        .await
        .expect("scalar wire value registers");
    let admitted = manager
        .lookup::<ConfigProbe<StringConfig>>(&nebula_core::ScopeLevel::Global)
        .unwrap()
        .config();
    assert_eq!(*admitted, config);
}

#[test]
fn wrong_scalar_config_shape_fails_schema_before_typed_decoding() {
    let error = validate::<StringConfig>(json!(42)).unwrap_err();
    let report = std::error::Error::source(&error)
        .unwrap()
        .downcast_ref::<ValidationReport>()
        .expect("wrong scalar shape must fail schema validation, not serde decoding");
    assert!(
        report
            .errors()
            .any(|error| { error.code() == "type_mismatch" && error.path().as_str().is_empty() })
    );
}

impl<C: ResourceConfig> HasCredentialSlots for ConfigProbe<C> {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
    fn declares_credential_slots() -> bool {
        false
    }
}

impl<C: ResourceConfig> DeclaresDependencies for ConfigProbe<C> {
    fn dependencies() -> Dependencies {
        Dependencies::new()
    }
}

#[async_trait::async_trait]
impl<C: ResourceConfig> Provider for ConfigProbe<C> {
    type Config = C;
    type Instance = ();
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("test.config-foundation")
    }

    async fn create(&self, _config: &C, _ctx: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl<C: ResourceConfig> nebula_resource::ResidentProvider for ConfigProbe<C> {}

#[derive(Clone, Debug, Deserialize, Hash)]
struct PreparedConfig {
    host: String,
}

impl HasSchema for PreparedConfig {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        Schema::builder()
            .add(
                Field::string(field_key!("host"))
                    .required()
                    .with_transformer(Transformer::Replace {
                        from: "a".into(),
                        to: "aa".into(),
                    }),
            )
            .build()
    }
}

impl ResourceConfig for PreparedConfig {
    fn fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

fn mode_string_schema() -> Result<ValidSchema, ValidationReport> {
    Schema::builder()
        .add(Field::string(field_key!("mode")).required())
        .build()
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Hash)]
struct FactoryAConfig {
    mode: String,
}

impl HasSchema for FactoryAConfig {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        mode_string_schema()
    }
}

impl ResourceConfig for FactoryAConfig {
    fn fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

#[derive(Clone, Debug, Deserialize, Hash)]
struct EqualSchemaFactoryBConfig {
    mode: String,
}

impl HasSchema for EqualSchemaFactoryBConfig {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        mode_string_schema()
    }
}

impl ResourceConfig for EqualSchemaFactoryBConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.mode == "factory-b" {
            Ok(())
        } else {
            Err(Error::permanent(
                "factory B rejected config admitted by factory A",
            ))
        }
    }

    fn fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

#[derive(Clone, Debug, Deserialize, Hash)]
struct DifferentSchemaFactoryBConfig {
    mode: u64,
}

impl HasSchema for DifferentSchemaFactoryBConfig {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        Schema::builder()
            .add(Field::number(field_key!("mode")).integer().required())
            .build()
    }
}

impl ResourceConfig for DifferentSchemaFactoryBConfig {
    fn fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

#[derive(Clone, Debug, Deserialize, Hash)]
struct SecretConfig {
    token: String,
}

impl HasSchema for SecretConfig {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        Schema::builder()
            .add(Field::secret(field_key!("token")).required())
            .build()
    }
}

impl ResourceConfig for SecretConfig {
    fn fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

#[derive(Clone, Debug, Deserialize)]
struct RejectedConfig;

impl HasSchema for RejectedConfig {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        Err(
            nebula_schema::ValidationError::builder("test.invalid_config_schema")
                .message("private-catalog-marker")
                .build()
                .into(),
        )
    }
}

impl ResourceConfig for RejectedConfig {
    fn fingerprint(&self) -> u64 {
        0
    }
}

#[test]
fn invalid_config_schema_retains_typed_catalog_failure() {
    let error = factory::<RejectedConfig>()
        .metadata()
        .expect_err("invalid static schema must fail factory admission");
    let nebula_resource::MetadataBuildError::Definition(shared_error) = &error else {
        panic!("schema construction must produce the typed definition variant");
    };
    let report = std::error::Error::source(shared_error)
        .unwrap()
        .downcast_ref::<ValidationReport>()
        .unwrap();
    assert_eq!(
        report.errors().next().unwrap().code(),
        "test.invalid_config_schema"
    );
    assert!(!format!("{error:?} {error}").contains("private-catalog-marker"));
}

#[derive(Clone, Debug, Deserialize, Hash)]
struct Endpoint {
    host: String,
}

#[derive(Clone, Debug, Deserialize, Hash)]
struct NestedConfig {
    endpoint: Endpoint,
}

impl HasSchema for NestedConfig {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        Schema::builder()
            .add(Field::object(field_key!("endpoint")).add(Field::string(field_key!("host"))))
            .build()
    }
}

impl ResourceConfig for NestedConfig {
    fn fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

async fn register_config<C: ResourceConfig + serde::de::DeserializeOwned>(
    manager: &Manager,
    config: ResourceConfigInput,
) -> Result<nebula_resource::SlotIdentity, Error> {
    let expression_engine = nebula_expression::ExpressionEngine::new();
    let expected_slot_identity = nebula_resource::SlotIdentity::Unbound;
    factory::<C>()
        .register(
            manager,
            RegisterRequest {
                config,
                expr_engine: &expression_engine,
                slot_bindings: Vec::new(),
                scope: nebula_core::ScopeLevel::Global,
                recovery_gate: None,
            },
            &expected_slot_identity,
        )
        .await
}

fn authored_field(key: &str, expression: Expression) -> ResourceConfigInput {
    let mut config = AuthoredValue::object();
    config
        .insert(key, AuthoredValue::Expression(expression))
        .expect("fixture root is an object");
    ResourceConfigInput::authored(config)
}

#[tokio::test]
async fn template_result_cannot_introduce_undeclared_secret_fields() {
    let manager = Manager::new();
    let error = register_config::<NestedConfig>(
        &manager,
        authored_field(
            "endpoint",
            Expression::new("{{ {host: 'example', password: 'private-result-marker'} }}"),
        ),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("/endpoint/password"));
    assert!(error.to_string().contains("not declared"));
    assert!(!format!("{error:?} {error}").contains("private-result-marker"));
    assert!(!manager.contains(&ConfigProbe::<NestedConfig>::key()));
}

#[tokio::test]
async fn template_results_remain_data_even_when_they_look_like_templates() {
    let manager = Manager::new();
    register_config::<PreparedConfig>(
        &manager,
        authored_field(
            "host",
            Expression::new("{{ '{' + '{ $missing.host ' + '}' + '}' }}"),
        ),
    )
    .await
    .unwrap();
    let config = manager
        .lookup::<ConfigProbe<PreparedConfig>>(&nebula_core::ScopeLevel::Global)
        .unwrap()
        .config();
    assert_eq!(config.host, "{{ $missing.host }}");
}

#[derive(Clone, Debug, Deserialize)]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "expression policy fixture deliberately declares an empty object, not unit null"
)]
struct EmptyConfig {}
nebula_schema::impl_empty_has_schema!(EmptyConfig);
impl ResourceConfig for EmptyConfig {
    fn fingerprint(&self) -> u64 {
        0
    }
}

#[test]
fn empty_record_rejects_literal_unknown_field_without_exposing_value() {
    let canary = "PRIVATE_EMPTY_RECORD_PASSWORD_CANARY";
    let error = validate::<EmptyConfig>(json!({
        "password": canary,
    }))
    .expect_err("an empty record must reject every supplied key");

    assert_eq!(error.kind(), &nebula_resource::ErrorKind::Permanent);
    let display = error.to_string();
    let debug = format!("{error:?}");
    let source = std::error::Error::source(&error)
        .map(ToString::to_string)
        .unwrap_or_default();
    assert!(display.contains("/password"));
    assert!(display.contains("not declared"));
    for rendered in [&display, &debug, &source] {
        assert!(!rendered.contains(canary));
    }
}

#[tokio::test]
async fn templates_require_a_permitting_field_declaration() {
    let manager = Manager::new();
    let error = register_config::<EmptyConfig>(
        &manager,
        authored_field("host", Expression::new("{{ $missing.host }}")),
    )
    .await
    .unwrap_err();
    let report = std::error::Error::source(&error)
        .unwrap()
        .downcast_ref::<ValidationReport>()
        .unwrap();
    assert!(report.errors().any(|error| error.code() == "expression.forbidden" && error.path().as_str() == "/host"));
    assert!(!manager.contains(&ConfigProbe::<EmptyConfig>::key()));
}

#[tokio::test]
async fn data_config_uses_prepared_values_exactly_once() {
    let manager = Manager::new();
    register_config::<PreparedConfig>(&manager, ResourceConfigInput::data(json!({"host": "a"})))
        .await
        .unwrap();
    let config = manager
        .lookup::<ConfigProbe<PreparedConfig>>(&nebula_core::ScopeLevel::Global)
        .unwrap()
        .config();
    assert_eq!(
        config.host, "aa",
        "typed config must consume the prepared tree, not the original JSON"
    );
}

#[tokio::test]
async fn data_config_never_executes_template_like_strings() {
    let literal = "{{ $missing.host }}";
    let manager = Manager::new();
    register_config::<PreparedConfig>(
        &manager,
        ResourceConfigInput::data(json!({"host": literal})),
    )
    .await
    .unwrap();
    let config = manager
        .lookup::<ConfigProbe<PreparedConfig>>(&nebula_core::ScopeLevel::Global)
        .unwrap()
        .config();
    assert_eq!(config.host, literal);
}

#[tokio::test]
async fn data_config_never_executes_expression_shaped_objects() {
    let expression_source = "{{ $missing.host }}";
    let manager = Manager::new();
    let error = register_config::<PreparedConfig>(
        &manager,
        ResourceConfigInput::data(json!({
            "host": {"$expr": expression_source},
        })),
    )
    .await
    .expect_err("an expression envelope at data ingress is ordinary object data, not code");
    let report = std::error::Error::source(&error)
        .expect("schema rejection retains its source")
        .downcast_ref::<ValidationReport>()
        .expect("data object must fail schema validation before expression evaluation");
    assert!(
        report
            .errors()
            .any(|error| error.code() == "type_mismatch" && error.path().as_str() == "/host")
    );
    assert!(!manager.contains(&ConfigProbe::<PreparedConfig>::key()));
}

#[tokio::test]
async fn equal_schema_factory_rechecks_its_typed_config_contract() {
    let authored =
        AuthoredValue::from_data(json!({"mode": "factory-a"})).expect("fixture is bounded data");
    let factory_a = factory::<FactoryAConfig>();
    let factory_b = factory::<EqualSchemaFactoryBConfig>();
    let schema_a = factory_a
        .metadata()
        .expect("factory A metadata admits")
        .base()
        .schema();
    let schema_b = factory_b
        .metadata()
        .expect("factory B metadata admits")
        .base()
        .schema();
    assert_eq!(
        schema_a, schema_b,
        "the fixture schemas must be structural equals"
    );

    let resolved_by_a = schema_a
        .validate(authored.clone())
        .and_then(nebula_schema::ValidValues::resolve_data)
        .expect("factory A schema admits the source");
    assert_eq!(
        resolved_by_a
            .into_typed::<FactoryAConfig>()
            .expect("factory A proof decodes as factory A's type"),
        FactoryAConfig {
            mode: "factory-a".to_owned(),
        }
    );

    let manager = Manager::new();
    let expression_engine = nebula_expression::ExpressionEngine::new();
    let expected_slot_identity = nebula_resource::SlotIdentity::Unbound;
    let error = factory_b
        .register(
            &manager,
            RegisterRequest {
                config: ResourceConfigInput::authored(authored),
                expr_engine: &expression_engine,
                slot_bindings: Vec::new(),
                scope: nebula_core::ScopeLevel::Global,
                recovery_gate: None,
            },
            &expected_slot_identity,
        )
        .await
        .expect_err("factory A admission cannot bypass factory B's typed validation");

    assert!(
        error
            .to_string()
            .contains("factory B rejected config admitted by factory A")
    );
    assert!(!manager.contains(&ConfigProbe::<EqualSchemaFactoryBConfig>::key()));
}

#[tokio::test]
async fn different_schema_factory_rechecks_its_own_schema() {
    let authored =
        AuthoredValue::from_data(json!({"mode": "factory-a"})).expect("fixture is bounded data");
    let factory_a = factory::<FactoryAConfig>();
    let factory_b = factory::<DifferentSchemaFactoryBConfig>();
    let schema_a = factory_a
        .metadata()
        .expect("factory A metadata admits")
        .base()
        .schema();
    let schema_b = factory_b
        .metadata()
        .expect("factory B metadata admits")
        .base()
        .schema();
    assert_ne!(schema_a, schema_b, "the fixture schemas must differ");
    let resolved_by_a = schema_a
        .validate(authored.clone())
        .and_then(nebula_schema::ValidValues::resolve_data)
        .expect("factory A schema admits the source");
    assert_eq!(
        resolved_by_a
            .into_typed::<FactoryAConfig>()
            .expect("factory A proof decodes as factory A's type"),
        FactoryAConfig {
            mode: "factory-a".to_owned(),
        }
    );

    let manager = Manager::new();
    let expression_engine = nebula_expression::ExpressionEngine::new();
    let expected_slot_identity = nebula_resource::SlotIdentity::Unbound;
    let error = factory_b
        .register(
            &manager,
            RegisterRequest {
                config: ResourceConfigInput::authored(authored),
                expr_engine: &expression_engine,
                slot_bindings: Vec::new(),
                scope: nebula_core::ScopeLevel::Global,
                recovery_gate: None,
            },
            &expected_slot_identity,
        )
        .await
        .expect_err("factory A admission cannot bypass factory B's different schema");
    let report = std::error::Error::source(&error)
        .expect("schema rejection retains its source")
        .downcast_ref::<ValidationReport>()
        .expect("different schema must reject before typed decoding");
    assert!(
        report
            .errors()
            .any(|error| error.code() == "type_mismatch" && error.path().as_str() == "/mode")
    );
    assert!(!manager.contains(&ConfigProbe::<DifferentSchemaFactoryBConfig>::key()));
}

#[test]
fn declared_secret_is_not_a_resource_config_escape_hatch() {
    let error = validate::<SecretConfig>(json!({"token": "private-token-marker"}))
        .expect_err("even schema-declared secrets must use credential slots");
    assert_eq!(error.kind(), &nebula_resource::ErrorKind::Permanent);
    assert!(!format!("{error:?} {error}").contains("private-token-marker"));
}

#[tokio::test]
async fn template_config_preserves_typed_results_and_preparation() {
    let manager = Manager::new();
    register_config::<PreparedConfig>(
        &manager,
        authored_field("host", Expression::new("{{ 'a' }}")),
    )
    .await
    .unwrap();
    let config = manager
        .lookup::<ConfigProbe<PreparedConfig>>(&nebula_core::ScopeLevel::Global)
        .unwrap()
        .config();
    assert_eq!(config.host, "aa");
}
