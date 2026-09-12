//! Factory registration from literal JSON or explicitly authored values.
//!
//! The flow:
//!
//!   1. Preserve literal JSON as data, or resolve explicitly authored values.
//!   2. Deserialize `R::Config` from the resolved JSON.
//!   3. Schema-validate the resolved JSON against `<R::Config as HasSchema>::schema()`.
//!   4. Validate `slot_bindings` keys against `R::dependencies()` (rejects configs whose credential
//!      surface diverged from the workflow JSON).
//!   5. Dispatch into the typed `Manager::register<R>(...)`.

use std::{collections::HashMap, sync::Arc};

use nebula_core::{
    CredentialKey, DeclaresDependencies, Dependencies, ResourceKey, ScopeLevel, resource_key,
};
use nebula_expression::ExpressionEngine;
use nebula_resource::Resident;
use nebula_resource::{
    KindActivator, Manager, RegisterRequest, ResidentConfig, ResourceConfigInput, ResourceContext,
    ResourceFactory,
    error::Error,
    resource::{HasCredentialSlots, Provider, ResourceConfig, ResourceMetadataDraft},
    topology::resident::ResidentProvider,
};
use serde::Deserialize;
use serde_json::json;

async fn register_from_value(
    factory: impl ResourceFactory,
    manager: &Manager,
    expression_engine: &ExpressionEngine,
    config_json: serde_json::Value,
    slot_bindings: HashMap<String, CredentialKey>,
) -> Result<nebula_resource::SlotIdentity, Error> {
    let slot_bindings: Vec<_> = slot_bindings
        .into_iter()
        .map(|(slot_name, credential_key)| nebula_resource::SlotBinding {
            slot_name,
            credential_key,
            credential_id: None,
        })
        .collect();
    let expected_slot_identity = nebula_resource::SlotIdentity::from_bindings(
        slot_bindings
            .iter()
            .map(|binding| (binding.slot_name.as_str(), binding.credential_key.as_str())),
    );
    factory
        .register(
            manager,
            RegisterRequest {
                config: ResourceConfigInput::data(config_json),
                expr_engine: expression_engine,
                slot_bindings,
                scope: ScopeLevel::Global,
                recovery_gate: None,
            },
            &expected_slot_identity,
        )
        .await
}

fn postgres_factory() -> impl ResourceFactory {
    KindActivator::<Postgres, _, _>::new(
        || Postgres,
        || Resident::<Postgres>::new(ResidentConfig::default()),
    )
}

// ── Test resource ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, Deserialize)]
struct PgConfig {
    host: String,
    #[serde(default = "default_port")]
    port: u16,
}

fn default_port() -> u16 {
    5432
}

impl nebula_schema::HasSchema for PgConfig {
    fn schema() -> Result<nebula_schema::ValidSchema, nebula_schema::ValidationReport> {
        // Type declarations authorize expressions; the custom ResourceConfig
        // validator below owns the non-empty host policy exercised by this fixture.
        nebula_schema::Schema::builder()
            .add(nebula_schema::Field::string(nebula_schema::field_key!(
                "host"
            )))
            .add(nebula_schema::Field::number(nebula_schema::field_key!("port")).integer())
            .build()
    }
}

impl ResourceConfig for PgConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.host.is_empty() {
            Err(Error::permanent("host must not be empty"))
        } else {
            Ok(())
        }
    }

    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.host.hash(&mut h);
        self.port.hash(&mut h);
        h.finish()
    }
}

#[derive(Clone)]
struct Postgres;

#[async_trait::async_trait]
impl Provider for Postgres {
    type Config = PgConfig;
    type Instance = Arc<()>;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("phase9-pg")
    }

    async fn create(&self, _config: &PgConfig, _ctx: &ResourceContext) -> Result<Arc<()>, Error> {
        Ok(Arc::new(()))
    }

    async fn destroy(
        &self,
        _runtime: Arc<()>,
        _cx: nebula_resource::TeardownCx,
    ) -> Result<(), Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("phase9-pg"),
            "",
        )
    }
}

nebula_resource::no_credential_slots!(Postgres);

#[async_trait::async_trait]
impl ResidentProvider for Postgres {
    fn is_alive_sync(&self, _runtime: &Arc<()>) -> bool {
        true
    }
}

impl DeclaresDependencies for Postgres {
    fn dependencies() -> Dependencies {
        // Default: no slot fields. Tests that exercise slot binding
        // declare the slot via a dedicated fixture below.
        Dependencies::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

struct AdmissionResource<const CREDENTIALS: bool>;

#[async_trait::async_trait]
impl<const CREDENTIALS: bool> Provider for AdmissionResource<CREDENTIALS> {
    type Config = PgConfig;
    type Instance = ();
    type Topology = AdmissionTopology;

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("AdmissionResource"),
            "",
        )
    }

    fn key() -> ResourceKey {
        resource_key!("test.revoke-admission")
    }

    async fn create(&self, _config: &PgConfig, _ctx: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }
}

impl<const CREDENTIALS: bool> HasCredentialSlots for AdmissionResource<CREDENTIALS> {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
    fn declares_credential_slots() -> bool {
        CREDENTIALS
    }
    fn credential_slot_names() -> &'static [&'static str] {
        if CREDENTIALS { &["auth"] } else { &[] }
    }
}

impl<const CREDENTIALS: bool> DeclaresDependencies for AdmissionResource<CREDENTIALS> {
    fn dependencies() -> Dependencies {
        if CREDENTIALS {
            Dependencies::new().slot_field(nebula_core::SlotField {
                slot_key: "auth",
                default_id: "auth",
                kind: nebula_core::dependencies::SlotKind::Credential {
                    type_id: std::any::TypeId::of::<()>(),
                    type_name: std::any::type_name::<()>(),
                    key: nebula_core::credential_key!("test.revoke-admission"),
                },
                required: true,
                lazy: false,
                purpose: None,
            })
        } else {
            Dependencies::new()
        }
    }
}

struct AdmissionTopology {
    handles_revoke: bool,
}

impl<const CREDENTIALS: bool> nebula_resource::topology::Topology<AdmissionResource<CREDENTIALS>>
    for AdmissionTopology
{
    type Entry = ();

    fn try_reserve(
        &self,
        _store: &nebula_resource::topology::InstanceStore<()>,
    ) -> Result<nebula_resource::topology::Ticket, nebula_resource::topology::Unavailable> {
        Ok(nebula_resource::topology::Ticket::infallible())
    }

    async fn create_entry(
        &self,
        _resource: &AdmissionResource<CREDENTIALS>,
        _config: &PgConfig,
        _ctx: &ResourceContext,
        _retained: &nebula_resource::topology::RetainedStore<Self::Entry>,
    ) -> Result<nebula_resource::topology::CreatedEntry<()>, Error> {
        Ok(nebula_resource::topology::CreatedEntry::new(()))
    }

    fn entry_instance<'s>(&self, entry: &'s ()) -> &'s () {
        entry
    }
    fn into_owned_instance(&self, entry: ()) -> Option<()> {
        Some(entry)
    }
    fn handles_own_revoke(&self) -> bool {
        self.handles_revoke
    }
}

fn register_admission_resource<const CREDENTIALS: bool>(
    manager: &Manager,
    handles_revoke: bool,
) -> Result<(), Error> {
    manager.register(nebula_resource::RegistrationSpec {
        resource: AdmissionResource::<CREDENTIALS>,
        config: PgConfig {
            host: "localhost".into(),
            port: 5432,
        },
        scope: ScopeLevel::Global,
        slot_identity: nebula_resource::SlotIdentity::Unbound,
        topology: AdmissionTopology { handles_revoke },
        recovery_gate: None,
    })
}

#[tokio::test]
async fn revoke_admission_rejects_unbound_credentialed_custom_topology() {
    let manager = Manager::new();
    let error = register_admission_resource::<true>(&manager, false)
        .expect_err("declared credentials require a revoke policy before binding");
    assert_eq!(error.kind(), &nebula_resource::ErrorKind::Permanent);
    assert!(!manager.contains(&AdmissionResource::<true>::key()));
}

#[tokio::test]
async fn revoke_admission_rejects_resolved_credentialed_custom_topology() {
    let manager = Manager::new();
    let expression_engine = ExpressionEngine::new();
    let error = register_from_value(
        KindActivator::<AdmissionResource<true>, _, _>::new(
            || AdmissionResource,
            || AdmissionTopology {
                handles_revoke: false,
            },
        ),
        &manager,
        &expression_engine,
        json!({"host": "localhost"}),
        HashMap::from([(
            "auth".to_owned(),
            nebula_core::credential_key!("binding-identity"),
        )]),
    )
    .await
    .expect_err("resolved registration must reject unsupported revoke policy");
    assert_eq!(error.kind(), &nebula_resource::ErrorKind::Permanent);
    assert!(!manager.contains(&AdmissionResource::<true>::key()));
}

#[tokio::test]
async fn revoke_admission_allows_custom_topology_without_credentials() {
    let manager = Manager::new();
    register_admission_resource::<false>(&manager, false)
        .expect("no credentials need no revoke hook");
    assert!(manager.contains(&AdmissionResource::<false>::key()));
}

#[tokio::test]
async fn revoke_admission_preserves_explicit_custom_revoke_policy() {
    let manager = Manager::new();
    register_admission_resource::<true>(&manager, true).expect("custom revoke policy is supported");
    assert!(manager.contains(&AdmissionResource::<true>::key()));
}

#[tokio::test]
async fn register_from_value_resolves_template_and_registers() {
    let manager = Manager::new();
    let engine = ExpressionEngine::new();
    let expected_slot_identity = nebula_resource::SlotIdentity::Unbound;

    let mut config = nebula_schema::AuthoredValue::object();
    config
        .insert(
            "host",
            nebula_schema::AuthoredValue::Expression(nebula_schema::Expression::template(
                "db-{{ \"example.com\" }}",
            )),
        )
        .unwrap();
    config.insert_data("port", json!(5432)).unwrap();

    postgres_factory()
        .register(
            &manager,
            RegisterRequest {
                config: ResourceConfigInput::authored(config),
                expr_engine: &engine,
                slot_bindings: Vec::new(),
                scope: ScopeLevel::Global,
                recovery_gate: None,
            },
            &expected_slot_identity,
        )
        .await
        .expect("explicitly authored template registration must succeed");

    assert!(
        manager.contains(&Postgres::key()),
        "resource must be registered after register_from_value"
    );

    // The rendered config must be *observable*, not merely "registered":
    // assert the `{{ "example.com" }}` template was actually resolved into
    // the stored `PgConfig` (not passed through verbatim or dropped). This
    // pins that `register_from_value` threads the resolved config all the
    // way into the installed registry row through the collapsed
    // `RegistrationSpec` funnel.
    let managed = manager
        .lookup::<Postgres>(&ScopeLevel::Global)
        .expect("registered resident row must be resolvable");
    let stored = managed.config();
    assert_eq!(
        stored.host, "db-example.com",
        "the `{{{{ \"example.com\" }}}}` template must have rendered into the stored host"
    );
    assert_eq!(stored.port, 5432, "non-templated field must round-trip");
}

#[tokio::test]
async fn register_from_value_validates_schema_failure() {
    let manager = Manager::new();
    let engine = ExpressionEngine::new();

    // The config now declares the fields that authorize template evaluation.
    // A wrong literal type is rejected by schema admission before deserialization.
    let config_json = json!({
        "host": 12345,
        "port": 5432,
    });

    let err = register_from_value(
        postgres_factory(),
        &manager,
        &engine,
        config_json,
        HashMap::new(),
    )
    .await
    .expect_err("must reject ill-typed config");

    let report = std::error::Error::source(&err)
        .unwrap()
        .downcast_ref::<nebula_schema::ValidationReport>()
        .unwrap();
    assert!(
        report
            .errors()
            .any(|error| error.code() == "type_mismatch" && error.path().as_str() == "/host")
    );
    assert!(!manager.contains(&Postgres::key()));
}

#[tokio::test]
async fn register_from_value_resourceconfig_validate_fires() {
    let manager = Manager::new();
    let engine = ExpressionEngine::new();

    // Empty `host` deserializes fine but fails ResourceConfig::validate().
    let config_json = json!({
        "host": "",
        "port": 5432,
    });

    let err = register_from_value(
        postgres_factory(),
        &manager,
        &engine,
        config_json,
        HashMap::new(),
    )
    .await
    .expect_err("must reject empty host");

    assert!(
        err.to_string().contains("host must not be empty"),
        "expected ResourceConfig::validate to surface, got: {err}"
    );
}

#[tokio::test]
async fn register_from_value_unknown_slot_binding_rejected() {
    let manager = Manager::new();
    let engine = ExpressionEngine::new();

    // `Postgres` declares no credential slots; supplying a binding
    // for `auth` is a misconfiguration that must be caught at register
    // time rather than as a confusing rotation no-op later.
    let mut bindings = HashMap::new();
    bindings.insert("auth".to_owned(), CredentialKey::new("db_auth").unwrap());

    let err = register_from_value(
        postgres_factory(),
        &manager,
        &engine,
        json!({"host": "example.com",
            "port": 5432}),
        bindings,
    )
    .await
    .expect_err("unknown slot must be rejected");

    let msg = err.to_string();
    assert!(
        msg.contains("auth") && msg.contains("does not match any declared credential slot"),
        "expected slot-binding rejection message, got: {msg}"
    );
}

// ── Registration consistency check: HasCredentialSlots vs
//    DeclaresDependencies drift ───────────────────────────────────────────

/// A hand-written resource whose two self-reported slot signals
/// deliberately disagree: `declares_credential_slots()` says `true` but
/// `DeclaresDependencies::dependencies()` declares zero credential slot
/// fields. Models the drift a hand-written pair can accumulate that
/// `#[derive(Resource)]` structurally cannot (it emits both from the same
/// `#[credential]` field list, so they can never disagree).
#[derive(Clone)]
struct DriftedSlotSignals;

#[async_trait::async_trait]
impl Provider for DriftedSlotSignals {
    type Config = PgConfig;
    type Instance = Arc<()>;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("phase9-drifted-slot-signals")
    }

    async fn create(&self, _config: &PgConfig, _ctx: &ResourceContext) -> Result<Arc<()>, Error> {
        Ok(Arc::new(()))
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("phase9-drifted-slot-signals"),
            "",
        )
    }
}

impl HasCredentialSlots for DriftedSlotSignals {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }

    // Deliberately drifted from `dependencies()` below (which declares NO
    // credential slot fields) — the contradiction under test.
    fn declares_credential_slots() -> bool {
        true
    }
}

#[async_trait::async_trait]
impl ResidentProvider for DriftedSlotSignals {
    fn is_alive_sync(&self, _runtime: &Arc<()>) -> bool {
        true
    }
}

impl DeclaresDependencies for DriftedSlotSignals {
    fn dependencies() -> Dependencies {
        // Deliberately empty — disagrees with `declares_credential_slots()`
        // above.
        Dependencies::new()
    }
}

#[tokio::test]
async fn register_from_value_rejects_drifted_slot_signals() {
    let manager = Manager::new();
    let engine = ExpressionEngine::new();

    let err = register_from_value(
        KindActivator::<DriftedSlotSignals, _, _>::new(
            || DriftedSlotSignals,
            || Resident::new(ResidentConfig::default()),
        ),
        &manager,
        &engine,
        json!({"host": "example.com", "port": 5432}),
        HashMap::new(),
    )
    .await
    .expect_err("a HasCredentialSlots vs DeclaresDependencies contradiction must be rejected");

    let msg = err.to_string();
    assert!(
        msg.contains("drifted apart"),
        "expected the slot-signal drift rejection message, got: {msg}"
    );
}

// ── Registration consistency check: HasCredentialSlots vs its own
//    credential_slot_names() drift ────────────────────────────────────────

/// A hand-written resource whose `declares_credential_slots()` and
/// `credential_slot_names()` disagree: `declares_credential_slots()` says
/// `true`, but `credential_slot_names()` keeps the empty trait default
/// (never overridden). `DeclaresDependencies` agrees with
/// `declares_credential_slots()`, so this fixture passes the
/// `HasCredentialSlots` vs `DeclaresDependencies` check above and isolates
/// the separate `declares_credential_slots()` vs `credential_slot_names()`
/// cross-check.
#[derive(Clone)]
struct DriftedSlotNames;

#[async_trait::async_trait]
impl Provider for DriftedSlotNames {
    type Config = PgConfig;
    type Instance = Arc<()>;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("phase9-drifted-slot-names")
    }

    async fn create(&self, _config: &PgConfig, _ctx: &ResourceContext) -> Result<Arc<()>, Error> {
        Ok(Arc::new(()))
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("phase9-drifted-slot-names"),
            "",
        )
    }
}

impl HasCredentialSlots for DriftedSlotNames {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }

    fn declares_credential_slots() -> bool {
        true
    }

    // Deliberately left at the empty trait default — disagrees with
    // `declares_credential_slots() -> true` above.
}

#[async_trait::async_trait]
impl ResidentProvider for DriftedSlotNames {
    fn is_alive_sync(&self, _runtime: &Arc<()>) -> bool {
        true
    }
}

impl DeclaresDependencies for DriftedSlotNames {
    fn dependencies() -> Dependencies {
        // Agrees with `declares_credential_slots() -> true`, so the
        // HasCredentialSlots-vs-DeclaresDependencies check passes and this
        // fixture isolates the names-vs-declares cross-check instead.
        Dependencies::new().slot_field(nebula_core::SlotField {
            slot_key: "db",
            default_id: "db",
            kind: nebula_core::dependencies::SlotKind::Credential {
                type_id: std::any::TypeId::of::<()>(),
                type_name: std::any::type_name::<()>(),
                key: nebula_core::credential_key!("phase9.drifted-names"),
            },
            required: true,
            lazy: false,
            purpose: None,
        })
    }
}

#[tokio::test]
async fn register_from_value_rejects_drifted_slot_names() {
    let manager = Manager::new();
    let engine = ExpressionEngine::new();

    let err = register_from_value(
        KindActivator::<DriftedSlotNames, _, _>::new(
            || DriftedSlotNames,
            || Resident::new(ResidentConfig::default()),
        ),
        &manager,
        &engine,
        json!({"host": "example.com", "port": 5432}),
        HashMap::new(),
    )
    .await
    .expect_err(
        "a declares_credential_slots() vs credential_slot_names() contradiction must be rejected",
    );

    let msg = err.to_string();
    assert!(
        msg.contains("drifted apart"),
        "expected the slot-signal drift rejection message, got: {msg}"
    );
}

#[tokio::test]
async fn register_from_value_passthrough_no_templates() {
    // Plain JSON with no `{{ }}` markers — the engine fast-path is
    // exercised; resolution is a no-op walk.
    let manager = Manager::new();
    let engine = ExpressionEngine::new();
    let config_json = json!({
        "host": "static.example.com",
        "port": 5432,
    });

    register_from_value(
        postgres_factory(),
        &manager,
        &engine,
        config_json,
        HashMap::new(),
    )
    .await
    .expect("register_from_value must succeed for plain JSON");

    assert!(manager.contains(&Postgres::key()));
}

// ── Union (enum) resource config: serde tagged wire → registration ───────────
//
// A `Provider` whose `Config` is a `#[derive(Schema)]` enum has a tagged-union
// schema. Factory registration ingests the operator's serde external wire
// (`{"Variant": payload}`) through `values_from_wire`, so the schema pass and the
// closed-set guard see the union's declared root key, and the same wire still
// deserializes into the enum directly. This is the resource half of the value-layer
// union bridge — a real first-party union consumer through real registration.

#[derive(Clone, Debug, PartialEq, Deserialize, nebula_schema::Schema)]
enum CacheBackendConfig {
    Memory { capacity: u64 },
    Redis { url: String },
}

impl ResourceConfig for CacheBackendConfig {
    fn validate(&self) -> Result<(), Error> {
        match self {
            Self::Memory { capacity } if *capacity == 0 => {
                Err(Error::permanent("memory capacity must be non-zero"))
            },
            Self::Redis { url } if url.is_empty() => {
                Err(Error::permanent("redis url must not be empty"))
            },
            _ => Ok(()),
        }
    }

    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        match self {
            Self::Memory { capacity } => {
                0u8.hash(&mut h);
                capacity.hash(&mut h);
            },
            Self::Redis { url } => {
                1u8.hash(&mut h);
                url.hash(&mut h);
            },
        }
        h.finish()
    }
}

#[derive(Clone)]
struct CacheBackend;

#[async_trait::async_trait]
impl Provider for CacheBackend {
    type Config = CacheBackendConfig;
    type Instance = Arc<()>;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("union-cache")
    }

    async fn create(
        &self,
        _config: &CacheBackendConfig,
        _ctx: &ResourceContext,
    ) -> Result<Arc<()>, Error> {
        Ok(Arc::new(()))
    }

    async fn destroy(
        &self,
        _runtime: Arc<()>,
        _cx: nebula_resource::TeardownCx,
    ) -> Result<(), Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("union-cache"),
            "",
        )
    }
}

nebula_resource::no_credential_slots!(CacheBackend);

#[async_trait::async_trait]
impl ResidentProvider for CacheBackend {
    fn is_alive_sync(&self, _runtime: &Arc<()>) -> bool {
        true
    }
}

impl DeclaresDependencies for CacheBackend {
    fn dependencies() -> Dependencies {
        Dependencies::new()
    }
}

fn cache_backend_factory() -> impl ResourceFactory {
    KindActivator::<CacheBackend, _, _>::new(
        || CacheBackend,
        || Resident::<CacheBackend>::new(ResidentConfig::default()),
    )
}

#[tokio::test]
async fn register_from_value_accepts_union_config_external_wire() {
    let manager = Manager::new();
    let engine = ExpressionEngine::new();

    // serde external data-variant wire: `{"Memory": {"capacity": 100}}`. Ingress
    // folds it into the union envelope, validation passes, and the enum is stored.
    let config_json = json!({ "Memory": { "capacity": 100 } });

    register_from_value(
        cache_backend_factory(),
        &manager,
        &engine,
        config_json,
        HashMap::new(),
    )
    .await
    .expect("union config registers via serde external wire");

    let managed = manager
        .lookup::<CacheBackend>(&ScopeLevel::Global)
        .expect("registered union-config row must resolve");
    assert_eq!(
        *managed.config(),
        CacheBackendConfig::Memory { capacity: 100 },
        "the serde external wire must deserialize into the active union variant"
    );
}

#[tokio::test]
async fn register_from_value_rejects_unknown_union_variant() {
    let manager = Manager::new();
    let engine = ExpressionEngine::new();

    // `Nope` is not a declared variant — ingress rejects it before validation.
    let err = register_from_value(
        cache_backend_factory(),
        &manager,
        &engine,
        json!({ "Nope": {} }),
        HashMap::new(),
    )
    .await
    .expect_err("an unknown union variant must be rejected at registration");

    // The top-level message names the operation; the structured cause (the
    // unknown-variant detail) is preserved on the source chain via
    // `.with_source(e)` rather than flattened into the message. Walk the whole
    // chain so the rejection detail must survive somewhere in it.
    let mut chain = err.to_string();
    let mut source = std::error::Error::source(&err);
    while let Some(cause) = source {
        chain.push_str(": ");
        chain.push_str(&cause.to_string());
        source = cause.source();
    }
    assert!(
        chain.contains("unknown_variant") || chain.contains("Nope"),
        "expected an unknown-variant rejection in the error chain, got: {chain}"
    );
}

#[tokio::test]
async fn register_from_value_rejects_inlined_field_in_union_variant_payload() {
    let manager = Manager::new();
    let engine = ExpressionEngine::new();

    // `capacity` is declared by the `Memory` variant; `secret_token` is not. The
    // closed-set guard must signal it (serde would otherwise silently drop the
    // unknown field), so a secret cannot be inlined into a union variant payload —
    // the union's equivalent of the record top-level closed-set guard.
    let err = register_from_value(
        cache_backend_factory(),
        &manager,
        &engine,
        json!({ "Memory": { "capacity": 100, "secret_token": "leak" } }),
        HashMap::new(),
    )
    .await
    .expect_err("an undeclared field in a union variant payload must be rejected");

    let msg = err.to_string();
    assert!(
        msg.contains("secret_token") && msg.contains("not declared"),
        "expected a union-variant-payload closed-set rejection naming the inlined key, got: {msg}"
    );
}
