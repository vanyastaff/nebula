use std::{
    any::TypeId,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use nebula_core::{Dependencies, ResourceKey, ResourceRequirement, resource_key};
use nebula_error::{Classify, ErrorCategory};
use nebula_expression::ExpressionEngine;

use super::*;
use crate::{
    Manager, Resident, ScopeLevel,
    error::Error as ResourceError,
    resource::{Provider, ResourceConfig, ResourceMetadataDraft},
    topology::resident::{self, ResidentProvider},
};

// ── Minimal test resource ────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct TestError(String);

impl std::fmt::Display for TestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TestError {}

impl From<TestError> for ResourceError {
    fn from(e: TestError) -> Self {
        ResourceError::transient(e.0)
    }
}

#[derive(Clone, Debug, serde::Deserialize, nebula_schema::Schema)]
struct TestConfig {
    #[serde(default)]
    #[field(default = "")]
    name: String,
}

impl ResourceConfig for TestConfig {
    fn validate(&self) -> Result<(), ResourceError> {
        if self.name.is_empty() {
            return Err(ResourceError::permanent("name must not be empty"));
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.name.hash(&mut h);
        h.finish()
    }
}

#[derive(Clone)]
struct TestRes {
    create_counter: Arc<AtomicU64>,
}

struct TestResourceDependency;

impl TestRes {
    fn new(create_counter: Arc<AtomicU64>) -> Self {
        Self { create_counter }
    }
}

#[async_trait::async_trait]
impl Provider for TestRes {
    type Config = TestConfig;
    type Instance = Arc<AtomicU64>;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("test-factory-res")
    }

    async fn create(
        &self,
        _config: &TestConfig,
        _ctx: &crate::ResourceContext,
    ) -> Result<Arc<AtomicU64>, ResourceError> {
        let id = self.create_counter.fetch_add(1, Ordering::Relaxed);
        Ok(Arc::new(AtomicU64::new(id)))
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            crate::metadata_name!("test-factory-res"),
            String::new(),
        )
    }
}

impl nebula_core::DeclaresDependencies for TestRes {
    fn dependencies() -> Dependencies {
        Dependencies::new().resource(ResourceRequirement::new(
            resource_key!("test-factory-dependency"),
            TypeId::of::<TestResourceDependency>(),
            std::any::type_name::<TestResourceDependency>(),
        ))
    }
}

crate::no_credential_slots!(TestRes);

#[async_trait::async_trait]
impl ResidentProvider for TestRes {
    fn is_alive_sync(&self, runtime: &Arc<AtomicU64>) -> bool {
        runtime.load(Ordering::Relaxed) < u64::MAX
    }
}

fn test_factory(create_counter: Arc<AtomicU64>) -> Arc<dyn ResourceFactory> {
    Arc::new(KindActivator::<TestRes, _, _>::new(
        move || TestRes::new(create_counter.clone()),
        || Resident::<TestRes>::new(resident::config::Config::default()),
    ))
}

#[cfg(feature = "rotation")]
#[derive(Clone)]
struct BoundTestRes {
    slot: Arc<crate::SlotCell<nebula_credential::CredentialGuard<u64>>>,
    projection_supported: bool,
}

#[cfg(feature = "rotation")]
impl BoundTestRes {
    fn new() -> Self {
        Self {
            slot: Arc::new(crate::SlotCell::empty()),
            projection_supported: true,
        }
    }

    fn without_projection() -> Self {
        Self {
            slot: Arc::new(crate::SlotCell::empty()),
            projection_supported: false,
        }
    }
}

#[cfg(feature = "rotation")]
#[async_trait::async_trait]
impl Provider for BoundTestRes {
    type Config = TestConfig;
    type Instance = ();
    type Topology = Resident<Self>;

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(Self::key(), crate::metadata_name!("BoundTestRes"), "")
    }

    fn key() -> ResourceKey {
        resource_key!("test-factory-bound-res")
    }

    async fn create(
        &self,
        _config: &TestConfig,
        _ctx: &crate::ResourceContext,
    ) -> Result<(), ResourceError> {
        Ok(())
    }
}

#[cfg(feature = "rotation")]
impl crate::HasCredentialSlots for BoundTestRes {
    fn credential_slot_epoch(&self) -> u64 {
        self.slot.generation()
    }

    fn declares_credential_slots() -> bool {
        true
    }

    fn credential_slot_names() -> &'static [&'static str] {
        &["auth"]
    }

    fn supports_credential_slot_projection(&self, slot: &str) -> bool {
        self.projection_supported && slot == "auth"
    }

    fn credential_slot_projection(
        &self,
        slot: &str,
    ) -> Option<(u64, Option<nebula_credential::CredentialGuardMetadata>)> {
        (self.projection_supported && slot == "auth").then(|| self.slot.projection_snapshot())
    }

    fn install_credential_slot_at_generation(
        &self,
        slot: &str,
        guard: nebula_credential::ErasedCredentialGuard,
        expected_generation: u64,
    ) -> Result<crate::SlotUpdate, crate::SlotInstallError> {
        if !self.projection_supported || slot != "auth" {
            return Err(crate::SlotInstallError::UnknownSlot);
        }
        let metadata = guard.metadata().clone();
        let guard = guard
            .into_typed::<u64>()
            .map_err(|_| crate::SlotInstallError::CredentialTypeMismatch)?;
        self.slot
            .install_projected_at_generation(expected_generation, metadata, Arc::new(guard))
    }

    fn fence_credential_slot_at_generation(
        &self,
        slot: &str,
        expected_generation: u64,
        fence: &mut dyn FnMut(),
    ) -> Result<(), crate::SlotInstallError> {
        if !self.projection_supported || slot != "auth" {
            return Err(crate::SlotInstallError::UnknownSlot);
        }
        self.slot
            .fence_projection_at_generation(expected_generation, fence)
    }

    fn install_credential_slot(
        &self,
        slot: &str,
        guard: nebula_credential::ErasedCredentialGuard,
    ) -> Result<crate::SlotUpdate, crate::SlotInstallError> {
        if !self.projection_supported || slot != "auth" {
            return Err(crate::SlotInstallError::UnknownSlot);
        }
        let metadata = guard.metadata().clone();
        let guard = guard
            .into_typed::<u64>()
            .map_err(|_| crate::SlotInstallError::CredentialTypeMismatch)?;
        self.slot.install_projected(metadata, Arc::new(guard))
    }

    fn revoke_credential_slot(
        &self,
        slot: &str,
    ) -> Result<crate::SlotUpdate, crate::SlotInstallError> {
        if !self.projection_supported || slot != "auth" {
            return Err(crate::SlotInstallError::UnknownSlot);
        }
        Ok(self.slot.revoke())
    }
}

#[cfg(feature = "rotation")]
impl nebula_core::DeclaresDependencies for BoundTestRes {
    fn dependencies() -> Dependencies {
        Dependencies::new().slot_field(nebula_core::SlotField {
            slot_key: "auth",
            default_id: "auth",
            kind: nebula_core::dependencies::SlotKind::Credential {
                type_id: TypeId::of::<()>(),
                type_name: std::any::type_name::<()>(),
                key: nebula_core::credential_key!("test.factory-credential"),
            },
            required: true,
            lazy: false,
            purpose: None,
        })
    }
}

#[cfg(feature = "rotation")]
#[async_trait::async_trait]
impl ResidentProvider for BoundTestRes {}

#[cfg(feature = "rotation")]
struct DivergentIdentityFactory {
    inner: Arc<dyn ResourceFactory>,
}

#[cfg(feature = "rotation")]
impl private::Sealed for DivergentIdentityFactory {}

#[cfg(feature = "rotation")]
impl ResourceFactory for DivergentIdentityFactory {
    fn key(&self) -> ResourceKey {
        self.inner.key()
    }

    fn dependencies(&self) -> &Dependencies {
        self.inner.dependencies()
    }

    fn resource_type_id(&self) -> TypeId {
        self.inner.resource_type_id()
    }

    fn metadata(&self) -> Result<&ResourceMetadata, crate::MetadataBuildError> {
        self.inner.metadata()
    }

    fn validate(&self, config_json: serde_json::Value) -> Result<(), ResourceError> {
        self.inner.validate(config_json)
    }

    fn register<'a>(
        &'a self,
        manager: &'a Manager,
        request: RegisterRequest<'a>,
        expected_slot_identity: &'a SlotIdentity,
    ) -> BoxFut<'a, Result<SlotIdentity, ResourceError>> {
        self.register_with_bindings(
            manager,
            request,
            expected_slot_identity,
            RegistrationBindings::empty(),
        )
    }

    fn register_with_bindings<'a>(
        &'a self,
        manager: &'a Manager,
        request: RegisterRequest<'a>,
        expected_slot_identity: &'a SlotIdentity,
        registration_bindings: RegistrationBindings<'a>,
    ) -> BoxFut<'a, Result<SlotIdentity, ResourceError>> {
        Box::pin(async move {
            self.inner
                .register_with_bindings(
                    manager,
                    request,
                    expected_slot_identity,
                    registration_bindings,
                )
                .await?;
            Ok(SlotIdentity::Unbound)
        })
    }
}

fn request(expr_engine: &ExpressionEngine) -> RegisterRequest<'_> {
    RegisterRequest {
        config: ResourceConfigInput::data(serde_json::json!({ "name": "from-factory" })),
        expr_engine,
        slot_bindings: Vec::new(),
        slot_installs: Vec::new(),
        scope: ScopeLevel::Global,
        recovery_gate: None,
    }
}

#[test]
fn registration_debug_redacts_opaque_config_and_binding_payloads() {
    let engine = ExpressionEngine::with_cache_size(16);
    let mut request = request(&engine);
    request.config = ResourceConfigInput::data(serde_json::json!({
        "opaque_config_field": ["config_secret_sentinel", {"nested": "nested_secret_sentinel"}]
    }));
    request.slot_bindings.push(SlotBinding {
        slot_name: "slot_name_sentinel".to_owned(),
        credential_key: nebula_core::CredentialKey::new("credential_key_sentinel")
            .expect("valid test key"),
        credential_id: Some(nebula_credential::CredentialId::new()),
        credential_scope: Some(nebula_credential::TenantScope::new("org", "workspace")),
    });
    for debug in [format!("{request:?}"), format!("{request:#?}")] {
        for sensitive in [
            "opaque_config_field",
            "config_secret_sentinel",
            "nested_secret_sentinel",
            "slot_name_sentinel",
            "credential_key_sentinel",
        ] {
            assert!(
                !debug.contains(sensitive),
                "opaque registration data must not appear in Debug"
            );
        }
        assert!(debug.contains("RegisterRequest"));
        assert!(debug.contains("slot_binding_count"));
        assert!(debug.contains("Global"));
    }
}

// ── Factory introspection arm ────────────────────────────────────────────

#[test]
fn key_coherence_law() {
    let create_counter = Arc::new(AtomicU64::new(0));
    let factory = test_factory(create_counter);
    assert_eq!(
        factory.key(),
        TestRes::key(),
        "factory.key() must equal <R as Provider>::key() (key-coherence law)"
    );
}

#[test]
fn factory_exposes_exact_resource_contract() {
    let factory = test_factory(Arc::new(AtomicU64::new(0)));
    let [resource_requirement] = factory.dependencies().resources() else {
        panic!("factory must expose the resource's one declared dependency");
    };

    assert_eq!(factory.resource_type_id(), TypeId::of::<TestRes>());
    assert_eq!(
        resource_requirement.key,
        resource_key!("test-factory-dependency")
    );
    assert_eq!(
        resource_requirement.type_id,
        TypeId::of::<TestResourceDependency>()
    );
    assert_eq!(
        resource_requirement.type_name,
        std::any::type_name::<TestResourceDependency>()
    );
    assert!(resource_requirement.required);
    assert_eq!(resource_requirement.purpose, None);
    assert!(factory.dependencies().credentials().is_empty());
    assert!(factory.dependencies().slot_fields().is_empty());
}

#[test]
fn metadata_schema_matches_provider_schema() {
    let create_counter = Arc::new(AtomicU64::new(0));
    let factory = test_factory(create_counter);
    let md = factory.metadata().expect("valid test catalog definition");
    let expected =
        nebula_schema::schema_of::<TestConfig>().expect("test configuration has a valid schema");
    assert_eq!(
        md.base().schema(),
        &expected,
        "metadata().schema must derive from the same HasSchema as validate/register \
         (schema-single-source law)"
    );
}

// ── Construction arm ─────────────────────────────────────────────────────

#[tokio::test]
async fn known_kind_registers_against_manager() {
    let manager = Manager::new();
    let expr_engine = ExpressionEngine::with_cache_size(16);
    let create_counter = Arc::new(AtomicU64::new(0));

    let mut registry = ResourceActivatorRegistry::new();
    let prev = registry
        .insert("test-kind", test_factory(create_counter))
        .expect("test resource metadata admits");
    assert!(prev.is_none(), "no prior factory for a fresh kind");
    assert!(registry.contains("test-kind"));
    assert_eq!(registry.len(), 1);

    registry
        .register("test-kind", &manager, request(&expr_engine))
        .await
        .expect("known kind registers via the typed manager call");

    assert!(
        manager
            .get_any(&TestRes::key(), &ScopeLevel::Global)
            .is_some(),
        "registered resource must be resolvable in the manager"
    );
}

#[cfg(feature = "rotation")]
#[tokio::test]
async fn identity_mismatch_is_typed_and_rolls_back_manager_and_fanout_state() {
    let manager = Manager::new();
    let expression_engine = ExpressionEngine::with_cache_size(16);
    let credential_id = nebula_credential::CredentialId::new();
    let fanout_index = Arc::new(crate::ResourceFanoutIndex::new());
    let inner: Arc<dyn ResourceFactory> = Arc::new(KindActivator::<BoundTestRes, _, _>::new(
        BoundTestRes::new,
        || Resident::<BoundTestRes>::new(resident::config::Config::default()),
    ));
    let mut registry = ResourceActivatorRegistry::new();
    registry
        .insert(
            "test-divergent-identity",
            Arc::new(DivergentIdentityFactory { inner }),
        )
        .expect("typed fixture metadata admits");

    let error = registry
        .register_and_bind(
            "test-divergent-identity",
            &manager,
            RegisterRequest {
                config: ResourceConfigInput::data(serde_json::json!({ "name": "from-factory" })),
                expr_engine: &expression_engine,
                slot_bindings: vec![SlotBinding {
                    slot_name: "auth".to_owned(),
                    credential_key: nebula_core::credential_key!("test.factory-credential"),
                    credential_id: Some(credential_id),
                    credential_scope: Some(nebula_credential::TenantScope::new("org", "workspace")),
                }],
                slot_installs: Vec::new(),
                scope: ScopeLevel::Global,
                recovery_gate: None,
            },
            Some(&fanout_index),
        )
        .await
        .expect_err("a divergent erased result must fail in release builds");

    assert_eq!(Classify::category(&error), ErrorCategory::Internal);
    assert_eq!(
        Classify::code(&error).as_str(),
        "RESOURCE:FACTORY_IDENTITY_MISMATCH"
    );
    std::assert_matches!(
        error,
        RegistrarError::IdentityMismatch {
            ref kind,
            ref expected,
            actual: SlotIdentity::Unbound,
        } if kind == "test-divergent-identity" && !expected.is_unbound()
    );
    assert!(!manager.contains(&BoundTestRes::key()));
    assert!(fanout_index.affected(&credential_id).is_empty());
}

#[cfg(feature = "rotation")]
#[tokio::test]
async fn conflicting_duplicate_slot_bindings_fail_before_manager_publication() {
    let manager = Manager::new();
    let expression_engine = ExpressionEngine::with_cache_size(16);
    let first_credential_id = nebula_credential::CredentialId::new();
    let second_credential_id = nebula_credential::CredentialId::new();
    let fanout_index = Arc::new(crate::ResourceFanoutIndex::new());
    let mut registry = ResourceActivatorRegistry::new();
    registry
        .insert(
            "test-conflicting-bindings",
            Arc::new(KindActivator::<BoundTestRes, _, _>::new(
                BoundTestRes::new,
                || Resident::<BoundTestRes>::new(resident::config::Config::default()),
            )),
        )
        .expect("typed fixture metadata admits");

    let error = registry
        .register_and_bind(
            "test-conflicting-bindings",
            &manager,
            RegisterRequest {
                config: ResourceConfigInput::data(serde_json::json!({ "name": "from-factory" })),
                expr_engine: &expression_engine,
                slot_bindings: vec![
                    SlotBinding {
                        slot_name: "auth".to_owned(),
                        credential_key: nebula_core::credential_key!("test.credential-a"),
                        credential_id: Some(first_credential_id),
                        credential_scope: Some(nebula_credential::TenantScope::new(
                            "org",
                            "workspace",
                        )),
                    },
                    SlotBinding {
                        slot_name: "auth".to_owned(),
                        credential_key: nebula_core::credential_key!("test.credential-b"),
                        credential_id: Some(second_credential_id),
                        credential_scope: Some(nebula_credential::TenantScope::new(
                            "org",
                            "workspace",
                        )),
                    },
                ],
                slot_installs: Vec::new(),
                scope: ScopeLevel::Global,
                recovery_gate: None,
            },
            Some(&fanout_index),
        )
        .await
        .expect_err("one slot cannot resolve to two credential identities");

    std::assert_matches!(error, RegistrarError::Register { .. });
    assert!(!manager.contains(&BoundTestRes::key()));
    assert!(fanout_index.affected(&first_credential_id).is_empty());
    assert!(fanout_index.affected(&second_credential_id).is_empty());
}

#[cfg(feature = "rotation")]
#[tokio::test]
async fn rotation_binding_without_owner_scope_fails_before_publication() {
    let manager = Manager::new();
    let expression_engine = ExpressionEngine::with_cache_size(16);
    let credential_id = nebula_credential::CredentialId::new();
    let fanout_index = Arc::new(crate::ResourceFanoutIndex::new());
    let mut registry = ResourceActivatorRegistry::new();
    registry
        .insert(
            "test-missing-owner-scope",
            Arc::new(KindActivator::<BoundTestRes, _, _>::new(
                BoundTestRes::new,
                || Resident::<BoundTestRes>::new(resident::config::Config::default()),
            )),
        )
        .expect("typed fixture metadata admits");

    let error = registry
        .register_and_bind(
            "test-missing-owner-scope",
            &manager,
            RegisterRequest {
                config: ResourceConfigInput::data(serde_json::json!({ "name": "resource" })),
                expr_engine: &expression_engine,
                slot_bindings: vec![SlotBinding {
                    slot_name: "auth".to_owned(),
                    credential_key: nebula_core::credential_key!("test.factory-credential"),
                    credential_id: Some(credential_id),
                    credential_scope: None,
                }],
                slot_installs: Vec::new(),
                scope: ScopeLevel::Global,
                recovery_gate: None,
            },
            Some(&fanout_index),
        )
        .await
        .expect_err("rotation participation requires durable owner scope");

    std::assert_matches!(error, RegistrarError::Register { .. });
    assert!(!manager.contains(&BoundTestRes::key()));
    assert!(fanout_index.affected(&credential_id).is_empty());
}

#[cfg(feature = "rotation")]
#[tokio::test]
async fn rotation_binding_without_projection_ports_fails_before_publication() {
    let manager = Manager::new();
    let expression_engine = ExpressionEngine::with_cache_size(16);
    let credential_id = nebula_credential::CredentialId::new();
    let fanout_index = Arc::new(crate::ResourceFanoutIndex::new());
    let mut registry = ResourceActivatorRegistry::new();
    registry
        .insert(
            "test-missing-projection-ports",
            Arc::new(KindActivator::<BoundTestRes, _, _>::new(
                BoundTestRes::without_projection,
                || Resident::<BoundTestRes>::new(resident::config::Config::default()),
            )),
        )
        .expect("typed fixture metadata admits");

    let error = registry
        .register_and_bind(
            "test-missing-projection-ports",
            &manager,
            RegisterRequest {
                config: ResourceConfigInput::data(serde_json::json!({ "name": "resource" })),
                expr_engine: &expression_engine,
                slot_bindings: vec![SlotBinding {
                    slot_name: "auth".to_owned(),
                    credential_key: nebula_core::credential_key!("test.factory-credential"),
                    credential_id: Some(credential_id),
                    credential_scope: Some(nebula_credential::TenantScope::new("org", "workspace")),
                }],
                slot_installs: Vec::new(),
                scope: ScopeLevel::Global,
                recovery_gate: None,
            },
            Some(&fanout_index),
        )
        .await
        .expect_err("rotation participation requires complete projection ports");

    std::assert_matches!(error, RegistrarError::Register { .. });
    assert!(!manager.contains(&BoundTestRes::key()));
    assert!(fanout_index.affected(&credential_id).is_empty());
}

#[cfg(feature = "rotation")]
#[tokio::test]
async fn revoke_observed_before_staging_taints_the_row_at_publication() {
    let manager = Manager::new();
    let expression_engine = ExpressionEngine::with_cache_size(16);
    let credential_id = nebula_credential::CredentialId::new();
    let fanout_index = Arc::new(crate::ResourceFanoutIndex::new());
    let mut registry = ResourceActivatorRegistry::new();
    registry
        .insert(
            "test-staged-revoke",
            Arc::new(KindActivator::<BoundTestRes, _, _>::new(
                BoundTestRes::new,
                || Resident::<BoundTestRes>::new(resident::config::Config::default()),
            )),
        )
        .expect("typed fixture metadata admits");

    // Models the event arriving after credential resolution but before
    // register_and_bind inserts its staged reverse-index row.
    fanout_index.remember_revocation(credential_id);
    registry
        .register_and_bind(
            "test-staged-revoke",
            &manager,
            RegisterRequest {
                config: ResourceConfigInput::data(serde_json::json!({ "name": "resource" })),
                expr_engine: &expression_engine,
                slot_bindings: vec![SlotBinding {
                    slot_name: "auth".to_owned(),
                    credential_key: nebula_core::credential_key!("test.factory-credential"),
                    credential_id: Some(credential_id),
                    credential_scope: Some(nebula_credential::TenantScope::new("org", "workspace")),
                }],
                slot_installs: Vec::new(),
                scope: ScopeLevel::Global,
                recovery_gate: None,
            },
            Some(&fanout_index),
        )
        .await
        .expect("registration publishes the staged binding");
    let binding = fanout_index
        .affected(&credential_id)
        .into_iter()
        .next()
        .expect("published binding remains indexed");
    let managed = manager
        .lookup_any_for_slot_identity_structural(
            &binding.resource_key,
            &binding.scope,
            &binding.slot_identity,
        )
        .expect("published row is registered");
    assert!(
        managed.is_tainted(),
        "publication must not make a credential-revoked row acquirable"
    );
}

#[cfg(feature = "rotation")]
#[tokio::test]
async fn exact_replacement_publishes_only_successor_staged_binding() {
    let manager = Manager::new();
    let expression_engine = ExpressionEngine::with_cache_size(16);
    let old_credential_id = nebula_credential::CredentialId::new();
    let new_credential_id = nebula_credential::CredentialId::new();
    let old_fanout_index = Arc::new(crate::ResourceFanoutIndex::new());
    let new_fanout_index = Arc::new(crate::ResourceFanoutIndex::new());
    let mut registry = ResourceActivatorRegistry::new();
    registry
        .insert(
            "test-replacement-bindings",
            Arc::new(KindActivator::<BoundTestRes, _, _>::new(
                BoundTestRes::new,
                || Resident::<BoundTestRes>::new(resident::config::Config::default()),
            )),
        )
        .expect("typed fixture metadata admits");

    for (credential_id, fanout_index) in [
        (old_credential_id, &old_fanout_index),
        (new_credential_id, &new_fanout_index),
    ] {
        registry
            .register_and_bind(
                "test-replacement-bindings",
                &manager,
                RegisterRequest {
                    config: ResourceConfigInput::data(serde_json::json!({
                        "name": "from-factory"
                    })),
                    expr_engine: &expression_engine,
                    slot_bindings: vec![SlotBinding {
                        slot_name: "auth".to_owned(),
                        credential_key: nebula_core::credential_key!("test.shared-key"),
                        credential_id: Some(credential_id),
                        credential_scope: Some(nebula_credential::TenantScope::new(
                            "org",
                            "workspace",
                        )),
                    }],
                    slot_installs: Vec::new(),
                    scope: ScopeLevel::Global,
                    recovery_gate: None,
                },
                Some(fanout_index),
            )
            .await
            .expect("exact identity registration");
    }

    assert!(old_fanout_index.affected(&old_credential_id).is_empty());
    assert_eq!(new_fanout_index.affected(&new_credential_id).len(), 1);
    manager
        .remove(&BoundTestRes::key())
        .expect("registration attached its supplied fan-out index");
    assert!(
        new_fanout_index.affected(&new_credential_id).is_empty(),
        "removal before driver startup must prune the published binding"
    );
}

#[tokio::test]
async fn unknown_kind_is_typed_error_not_panic_not_silent() {
    let manager = Manager::new();
    let expr_engine = ExpressionEngine::with_cache_size(16);
    let create_counter = Arc::new(AtomicU64::new(0));

    let mut registry = ResourceActivatorRegistry::new();
    registry
        .insert("test-kind", test_factory(create_counter))
        .expect("test resource metadata admits");

    let err = registry
        .register("ghost", &manager, request(&expr_engine))
        .await
        .expect_err("unknown kind must NOT register and must NOT panic");

    match &err {
        RegistrarError::UnknownKind(kind) => assert_eq!(kind, "ghost"),
        other => panic!("expected UnknownKind(\"ghost\"), got {other:?}"),
    }

    assert!(
        manager
            .get_any(&TestRes::key(), &ScopeLevel::Global)
            .is_none(),
        "an unknown kind must never have touched a resource type"
    );
}

#[test]
fn unknown_kind_classifies_as_nonretryable_client_conflict() {
    let err = RegistrarError::UnknownKind("ghost".to_owned());

    assert_eq!(Classify::category(&err), ErrorCategory::Conflict);
    assert_ne!(Classify::category(&err), ErrorCategory::Internal);
    assert!(ErrorCategory::Conflict.is_client_error());
    assert!(!ErrorCategory::Conflict.is_server_error());
    assert!(!Classify::is_retryable(&err));
    assert!(!ErrorCategory::Conflict.is_default_retryable());
    assert!(Classify::retry_hint(&err).is_none());
    assert_eq!(
        Classify::code(&err).as_str(),
        "RESOURCE:FACTORY_UNKNOWN_KIND"
    );
}

#[test]
fn register_error_delegates_classification_to_inner() {
    let err = RegistrarError::Register {
        kind: "test-kind".to_owned(),
        source: ResourceError::permanent("schema validation failed"),
    };
    let inner = ResourceError::permanent("schema validation failed");
    assert_eq!(
        Classify::category(&err),
        Classify::category(&inner),
        "Register must delegate its category to the inner resource error"
    );
    assert_eq!(
        Classify::code(&err).as_str(),
        Classify::code(&inner).as_str()
    );
}

#[tokio::test]
async fn empty_registry_rejects_every_kind() {
    let manager = Manager::new();
    let expr_engine = ExpressionEngine::with_cache_size(16);
    let registry = ResourceActivatorRegistry::new();
    assert!(registry.is_empty());

    let err = registry
        .register("anything", &manager, request(&expr_engine))
        .await
        .expect_err("an empty allowlist is fail-closed");
    assert!(matches!(err, RegistrarError::UnknownKind(k) if k == "anything"));
}
