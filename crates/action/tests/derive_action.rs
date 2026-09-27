//! Integration tests for `#[derive(Action)]` macro (Variant A).
//!
//! Tests verify that the macro correctly emits the `Action` trait impl
//! plus a `FromWorkflowNode` factory body that resolves slot fields.

use nebula_action::{
    Action, ActionContext, ActionError, ActionFactory, ActionResult, MetadataVersion,
    StatelessAction, effect::ActionEffectContract,
};
use nebula_schema::HasSchema;

// -- No slot fields ---------------------------------------------------------

#[derive(Action)]
#[action(
    key = "test.no_cred",
    name = "No Cred",
    description = "no credentials",
    input = serde_json::Value,
    output = serde_json::Value
)]
struct NoCredAction;

impl StatelessAction for NoCredAction {
    async fn execute(
        &self,
        input: serde_json::Value,
        _: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

#[test]
fn no_credentials_returns_empty_slot_fields() {
    assert!(NoCredAction::dependencies().slot_fields().is_empty());
}

#[test]
fn no_resources_in_dependencies() {
    assert!(NoCredAction::dependencies().resources().is_empty());
}

#[test]
fn metadata_key_matches_attribute() {
    let factory = nebula_action::GenericStatelessFactory::<NoCredAction>::new()
        .expect("valid test catalog definition");
    let meta = factory.metadata();
    assert_eq!(meta.base().key().as_str(), "test.no_cred");
    assert_eq!(meta.base().name().to_owned(), "No Cred");
    assert_eq!(meta.base().description().to_owned(), "no credentials");
}

#[derive(Action)]
#[action(
    key = "test.no_external_effects",
    description = "explicit no-effect contract",
    input = serde_json::Value,
    output = serde_json::Value,
    no_external_effects
)]
struct NoExternalEffectsAction;

impl StatelessAction for NoExternalEffectsAction {
    async fn execute(
        &self,
        input: serde_json::Value,
        _: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

#[test]
fn effect_contract_requires_an_explicit_author_attestation() {
    let undeclared = nebula_action::GenericStatelessFactory::<NoCredAction>::new()
        .expect("valid undeclared definition");
    let declared = nebula_action::GenericStatelessFactory::<NoExternalEffectsAction>::new()
        .expect("valid no-effect definition");

    assert_eq!(
        undeclared.metadata().effect_contract(),
        &ActionEffectContract::Undeclared
    );
    assert_eq!(
        declared.metadata().effect_contract(),
        &ActionEffectContract::NoExternalEffects
    );
}

#[test]
fn input_schema_derives_from_input_via_schema_of() {
    // P3: there is no `Action::input_schema()` method. The action's
    // input schema is reached through the `Input: HasSchema` associated-type
    // bound via `nebula_schema::schema_of` — the single source of truth.
    let schema = nebula_schema::schema_of::<<NoCredAction as Action>::Input>()
        .expect("valid test catalog definition");
    let direct = <serde_json::Value as HasSchema>::schema().expect("valid test catalog definition");
    assert_eq!(schema, direct);
}

// -- Default name + description (omitted attrs) ----------------------------

#[derive(Action)]
#[action(
    key = "test.defaults",
    input = serde_json::Value,
    output = serde_json::Value
)]
/// Default action description.
struct DefaultsAction;

impl StatelessAction for DefaultsAction {
    async fn execute(
        &self,
        input: serde_json::Value,
        _: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

#[test]
fn name_defaults_to_struct_name() {
    let factory = nebula_action::GenericStatelessFactory::<DefaultsAction>::new()
        .expect("valid test catalog definition");
    let meta = factory.metadata();
    assert_eq!(meta.base().name().to_owned(), "DefaultsAction");
    assert_eq!(meta.base().description(), "Default action description.");
}

// -- Default version --------------------------------------------------------

#[derive(Action)]
#[action(
    key = "test.versioned",
    description = "Versioned action",
    version = "2.5.0",
    input = serde_json::Value,
    output = serde_json::Value
)]
struct VersionedAction;

impl StatelessAction for VersionedAction {
    async fn execute(
        &self,
        input: serde_json::Value,
        _: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

#[test]
fn explicit_version_is_propagated() {
    let factory = nebula_action::GenericStatelessFactory::<VersionedAction>::new()
        .expect("valid test catalog definition");
    let meta = factory.metadata();
    assert_eq!(meta.base().version().major, 2);
    assert_eq!(meta.base().version().minor, 5);
    assert_eq!(meta.base().version().patch, 0);
}

#[derive(Action)]
#[action(
    key = "test.full-semver",
    name = "Full SemVer",
    description = "Full version fixture",
    version = "2.5.0-rc.7+build.009",
    input = serde_json::Value,
    output = serde_json::Value
)]
struct FullSemverAction;

impl StatelessAction for FullSemverAction {
    async fn execute(
        &self,
        input: serde_json::Value,
        _: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        Ok(ActionResult::success(input))
    }
}

#[test]
fn macro_and_manual_metadata_preserve_the_exact_full_semver() {
    let generated = nebula_action::GenericStatelessFactory::<FullSemverAction>::new()
        .expect("valid generated definition");
    let version: MetadataVersion = "2.5.0-rc.7+build.009".parse().expect("valid full SemVer");
    let manual = nebula_action::InstanceFactory::new(
        nebula_action::ActionMetadataDraft::try_new(
            nebula_core::action_key!("test.full-semver"),
            "Full SemVer",
            "Full version fixture",
        )
        .expect("valid name")
        .with_version(version.clone()),
        FullSemverAction,
    )
    .expect("valid manual definition");

    assert_eq!(generated.metadata().base().version(), &version);
    assert_eq!(**generated.metadata(), **manual.metadata());
}

// -- Managed row fields -----------------------------------------------------

mod managed_row_fields {
    use std::{any::Any, future::Future, pin::Pin, sync::Arc};

    use nebula_action::{FromWorkflowNode, testing::TestContextBuilder};
    use nebula_core::{
        CoreError, ResourceKey, ScopeLevel, accessor::ResourceAccessor, node_key, resource_key,
        scope::Scope,
    };
    use nebula_resource::{
        AcquireOptions, ErrorKind, Manager, PinSlots, RegistrationSpec, Resident, ResidentConfig,
        ResourceConfig, ResourceContext, SlotIdentity,
        call::{Cost, Effect, ManagedRow, OpCx, OpError, Operation, SentState},
        resource::{Provider, ResourceMetadataDraft},
        topology::ResidentProvider,
    };
    use nebula_workflow::NodeDefinition;
    use tokio_util::sync::CancellationToken;

    use super::*;

    #[derive(Clone, nebula_schema::Schema)]
    struct Config;

    impl ResourceConfig for Config {
        fn fingerprint(&self) -> u64 {
            0
        }
    }

    macro_rules! resident {
        ($ty:ident, $key:literal, $value:expr) => {
            #[derive(Clone)]
            struct $ty;

            #[async_trait::async_trait]
            impl Provider for $ty {
                type Config = Config;
                type Instance = u64;
                type Topology = Resident<Self>;

                fn key() -> ResourceKey {
                    resource_key!($key)
                }

                fn metadata() -> ResourceMetadataDraft {
                    ResourceMetadataDraft::new(
                        Self::key(),
                        nebula_resource::metadata_name!($key),
                        "",
                    )
                }

                async fn create(
                    &self,
                    _: &Config,
                    _: &ResourceContext,
                ) -> Result<u64, nebula_resource::Error> {
                    Ok($value)
                }
            }

            nebula_resource::no_credential_slots!($ty);

            impl ResidentProvider for $ty {}
        };
    }

    resident!(Db, "derive.row.db", 7);
    resident!(Cache, "derive.row.cache", 9);

    #[derive(Action)]
    #[action(
        key = "test.managed_row_fields",
        name = "Managed row fields",
        description = "row facade slots",
        input = serde_json::Value,
        output = serde_json::Value,
        no_external_effects
    )]
    struct RowAction {
        #[resource]
        db: ManagedRow<Db>,
        #[resource(key = "cache")]
        cache: Option<ManagedRow<Cache>>,
    }

    impl StatelessAction for RowAction {
        async fn execute(
            &self,
            input: serde_json::Value,
            _: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<serde_json::Value>, ActionError> {
            Ok(ActionResult::success(input))
        }
    }

    /// Reads the instance in one free attempt.
    struct Read;

    impl<R: Provider<Instance = u64> + PinSlots> Operation<R> for Read {
        type Output = u64;
        const EFFECT: Effect = Effect::Read;

        async fn run(self, cx: &mut OpCx<'_, R>) -> Result<u64, OpError> {
            let attempt = cx.attempt(Cost::FREE).await?;
            let value = *attempt.instance();
            attempt.settle(SentState::Sent);
            Ok(value)
        }
    }

    /// Serves the manager's rows for the unbound identity at the global
    /// scope.
    struct RowsOf(Arc<Manager>);

    type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

    impl ResourceAccessor for RowsOf {
        fn has(&self, _key: &ResourceKey) -> bool {
            true
        }

        fn acquire_any(
            &self,
            _key: &ResourceKey,
        ) -> BoxFut<'_, Result<Box<dyn Any + Send + Sync>, CoreError>> {
            Box::pin(async {
                Err(CoreError::resource_unavailable(
                    "lease", "unused", false, None,
                ))
            })
        }

        fn try_acquire_any(
            &self,
            _key: &ResourceKey,
        ) -> BoxFut<'_, Result<Option<Box<dyn Any + Send + Sync>>, CoreError>> {
            Box::pin(async { Ok(None) })
        }

        fn managed_row_any(
            &self,
            key: &ResourceKey,
        ) -> Result<Box<dyn Any + Send + Sync>, CoreError> {
            self.0
                .managed_row_any(
                    key,
                    &ResourceContext::minimal(Scope::default(), CancellationToken::new()),
                    &AcquireOptions::default(),
                    &SlotIdentity::Unbound,
                )
                .map_err(|error| error.to_core_error())
        }

        fn try_managed_row_any(
            &self,
            key: &ResourceKey,
        ) -> Result<Option<Box<dyn Any + Send + Sync>>, CoreError> {
            match self.0.managed_row_any(
                key,
                &ResourceContext::minimal(Scope::default(), CancellationToken::new()),
                &AcquireOptions::default(),
                &SlotIdentity::Unbound,
            ) {
                Ok(row) => Ok(Some(row)),
                Err(error) if matches!(error.kind(), ErrorKind::NotFound) => Ok(None),
                Err(error) => Err(error.to_core_error()),
            }
        }
    }

    /// Serves ordinary rows from `manager` but refuses one selected row as
    /// temporarily unavailable.
    struct RetryingRows {
        manager: Arc<Manager>,
        retry_key: ResourceKey,
    }

    impl ResourceAccessor for RetryingRows {
        fn has(&self, _key: &ResourceKey) -> bool {
            true
        }

        fn acquire_any(
            &self,
            _key: &ResourceKey,
        ) -> BoxFut<'_, Result<Box<dyn Any + Send + Sync>, CoreError>> {
            Box::pin(async {
                Err(CoreError::resource_unavailable(
                    "lease", "unused", false, None,
                ))
            })
        }

        fn try_acquire_any(
            &self,
            _key: &ResourceKey,
        ) -> BoxFut<'_, Result<Option<Box<dyn Any + Send + Sync>>, CoreError>> {
            Box::pin(async { Ok(None) })
        }

        fn managed_row_any(
            &self,
            key: &ResourceKey,
        ) -> Result<Box<dyn Any + Send + Sync>, CoreError> {
            if key == &self.retry_key {
                return Err(CoreError::resource_unavailable(
                    key.to_string(),
                    "row temporarily unavailable",
                    true,
                    None,
                ));
            }
            RowsOf(Arc::clone(&self.manager)).managed_row_any(key)
        }

        fn try_managed_row_any(
            &self,
            key: &ResourceKey,
        ) -> Result<Option<Box<dyn Any + Send + Sync>>, CoreError> {
            if key == &self.retry_key {
                return Err(CoreError::resource_unavailable(
                    key.to_string(),
                    "row temporarily unavailable",
                    true,
                    None,
                ));
            }
            RowsOf(Arc::clone(&self.manager)).try_managed_row_any(key)
        }
    }

    fn register<R>(manager: &Manager, resource: R)
    where
        R: Provider<Config = Config, Topology = Resident<R>> + ResidentProvider + Clone,
    {
        manager
            .register(RegistrationSpec {
                resource,
                config: Config,
                scope: ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: Resident::new(ResidentConfig::default()),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register");
    }

    fn node() -> NodeDefinition {
        NodeDefinition::new(node_key!("rows"), "Rows", "test", "test.managed_row_fields")
            .expect("valid node")
            .with_resource_binding("db", "derive.row.db")
    }

    #[test]
    fn row_fields_declare_their_resources() {
        let deps = RowAction::dependencies();
        let slots = deps.slot_fields();
        assert_eq!(slots.len(), 2);
        let db = slots.iter().find(|s| s.slot_key == "db").expect("db slot");
        assert!(db.required && !db.lazy);
        assert!(matches!(
            &db.kind,
            nebula_core::SlotKind::Resource { key, type_id, .. }
                if *key == Db::key() && *type_id == std::any::TypeId::of::<Db>()
        ));
        let cache = slots
            .iter()
            .find(|s| s.slot_key == "cache")
            .expect("cache slot");
        assert!(!cache.required && !cache.lazy);
    }

    #[tokio::test]
    async fn row_fields_resolve_to_usable_facades() {
        let manager = Arc::new(Manager::new());
        register(&manager, Db);
        register(&manager, Cache);
        let context = TestContextBuilder::new()
            .build()
            .with_resources(Arc::new(RowsOf(Arc::clone(&manager))));

        let node = node().with_resource_binding("cache", "derive.row.cache");
        let action = RowAction::from_workflow_node(&node, &context)
            .await
            .expect("both rows resolve");
        assert_eq!(action.db.submit(Read).await.expect("db unit"), 7);
        let cache = action.cache.expect("bound cache row");
        assert_eq!(cache.submit(Read).await.expect("cache unit"), 9);
    }

    #[tokio::test]
    async fn an_optional_row_is_absent_unbound_and_fatal_when_bound_to_nothing() {
        let manager = Arc::new(Manager::new());
        register(&manager, Db);
        let context = TestContextBuilder::new()
            .build()
            .with_resources(Arc::new(RowsOf(Arc::clone(&manager))));

        let action = RowAction::from_workflow_node(&node(), &context)
            .await
            .expect("the optional row is absent");
        assert!(action.cache.is_none());

        let bound = node().with_resource_binding("cache", "derive.row.cache");
        let Err(error) = RowAction::from_workflow_node(&bound, &context).await else {
            panic!("an explicit binding to an unregistered row must fail");
        };
        assert!(matches!(error, ActionError::Fatal { .. }), "{error}");
    }

    #[tokio::test]
    async fn required_and_optional_rows_preserve_retryable_resolution_failures() {
        let manager = Arc::new(Manager::new());
        register(&manager, Db);

        let required_context =
            TestContextBuilder::new()
                .build()
                .with_resources(Arc::new(RetryingRows {
                    manager: Arc::clone(&manager),
                    retry_key: Db::key(),
                }));
        let Err(required_error) = RowAction::from_workflow_node(&node(), &required_context).await
        else {
            panic!("required row is temporarily unavailable");
        };
        assert!(
            matches!(required_error, ActionError::Retryable { .. }),
            "{required_error}"
        );

        let optional_context =
            TestContextBuilder::new()
                .build()
                .with_resources(Arc::new(RetryingRows {
                    manager,
                    retry_key: ResourceKey::new("cache").expect("valid default slot id"),
                }));
        let Err(optional_error) = RowAction::from_workflow_node(&node(), &optional_context).await
        else {
            panic!("optional row is temporarily unavailable, not absent");
        };
        assert!(
            matches!(optional_error, ActionError::Retryable { .. }),
            "{optional_error}"
        );
    }
}
