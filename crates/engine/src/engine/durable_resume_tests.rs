use super::*;

#[derive(serde::Serialize, serde::Deserialize, nebula_schema::Schema)]
enum RecordedUnionInput {
    Count { number: i64 },
}

struct UnionEcho {
    calls: Arc<AtomicU32>,
}

impl Action for UnionEcho {
    type Input = RecordedUnionInput;
    type Output = RecordedUnionInput;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            action_key!("exact.union_echo"),
            nebula_action::metadata_name!("Union echo"),
            "Preserve declared union input",
        )
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
    }

    fn dependencies() -> &'static Dependencies {
        EchoHandler::dependencies()
    }
}

impl StatelessAction for UnionEcho {
    async fn execute(
        &self,
        input: Self::Input,
        _: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ActionResult::success(input))
    }
}

fn union_echo_fixture() -> (WorkflowEngine, Arc<AtomicU32>, NodeDefinition) {
    let registry = Arc::new(ActionRegistry::new());
    let calls = Arc::new(AtomicU32::new(0));
    registry
        .register_stateless_instance(
            UnionEcho::metadata(),
            UnionEcho {
                calls: Arc::clone(&calls),
            },
        )
        .unwrap();
    let (engine, _) = make_engine(registry);
    let node =
        NodeDefinition::new(node_key!("union"), "Union", "exact", "exact.union_echo").unwrap();
    (engine, calls, node)
}

#[rstest::rstest]
#[case::literal(nebula_workflow::ParamValue::literal(serde_json::json!({
    "mode": "Count", "value": {"number": 7}
})))]
#[case::expression(nebula_workflow::ParamValue::expression("{{ $input }}"))]
#[tokio::test]
async fn recorded_union_internal_parameters_reach_typed_action(
    #[case] parameter: nebula_workflow::ParamValue,
) {
    let (engine, calls, node) = union_echo_fixture();
    let schema = nebula_schema::schema_of::<RecordedUnionInput>().unwrap();
    let node = node.with_parameter(schema.fields()[0].key().as_str(), parameter);
    let workflow = make_workflow(vec![node], vec![]);
    let stores = TestStores::new();
    let result = stores
        .execute_accepted_fixture(
            engine,
            &workflow,
            serde_json::json!({"mode": "Count", "value": {"number": 7}}),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(result.is_success(), "{result:?}");
    assert_eq!(
        result.node_output(&node_key!("union")),
        Some(&serde_json::to_value(RecordedUnionInput::Count { number: 7 }).unwrap())
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn raw_external_union_input_reaches_typed_action() {
    let (engine, calls, node) = union_echo_fixture();
    let workflow = make_workflow(vec![node], vec![]);
    let wire = serde_json::to_value(RecordedUnionInput::Count { number: 7 }).unwrap();
    let result = engine
        .execute_workflow(
            &crate::store_seam::single_tenant_scope(),
            &workflow,
            wire.clone(),
            ExecutionBudget::default(),
        )
        .await
        .unwrap();

    assert!(result.is_success(), "{result:?}");
    assert_eq!(result.node_output(&node_key!("union")), Some(&wire));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[derive(Debug)]
struct CorruptPlanCatalog {
    inner: Arc<dyn nebula_storage_port::PlanFlavorCatalog>,
}

#[derive(Debug)]
struct CatalogClock {
    entry: DateTime<Utc>,
    load_completed: std::sync::atomic::AtomicBool,
}

impl Clock for CatalogClock {
    fn now(&self) -> DateTime<Utc> {
        if self.load_completed.load(Ordering::SeqCst) {
            self.entry + chrono::Duration::seconds(100)
        } else {
            self.entry
        }
    }

    fn monotonic(&self) -> Instant {
        Instant::now()
    }
}

#[derive(Debug)]
struct ClockChangingCatalog {
    inner: Arc<dyn nebula_storage_port::PlanFlavorCatalog>,
    clock: Arc<CatalogClock>,
}

#[async_trait::async_trait]
impl nebula_storage_port::PlanFlavorCatalog for ClockChangingCatalog {
    async fn load_exact(
        &self,
        ids: nebula_storage_port::PlanFlavorRevisionIds,
    ) -> Result<
        nebula_storage_port::PlanFlavorRevisionRecord,
        nebula_storage_port::RevisionCatalogError,
    > {
        let record = self.inner.load_exact(ids).await?;
        self.clock.load_completed.store(true, Ordering::SeqCst);
        Ok(record)
    }
}

#[async_trait::async_trait]
impl nebula_storage_port::PlanFlavorCatalog for CorruptPlanCatalog {
    async fn load_exact(
        &self,
        ids: nebula_storage_port::PlanFlavorRevisionIds,
    ) -> Result<
        nebula_storage_port::PlanFlavorRevisionRecord,
        nebula_storage_port::RevisionCatalogError,
    > {
        let record = self.inner.load_exact(ids).await?;
        Ok(
            nebula_storage_port::PlanFlavorRevisionRecord::graph_v1_json(
                ids.plan(),
                nebula_storage_port::RevisionRecordBytes::try_from_vec(
                    b"secret-canary-corrupt-json".to_vec(),
                )
                .unwrap(),
                record.worker_flavor().clone(),
            ),
        )
    }
}

#[tokio::test]
async fn durable_budget_samples_prior_elapsed_before_catalog_io() {
    let runtime_registry = Arc::new(ActionRegistry::new());
    let (frozen, instantiations) = snapshot_registry_counted(&runtime_registry, "within-budget");
    let stores = TestStores::new();
    let (original_id, _) = install_snapshot_execution(&stores, &frozen).await;
    let (_, original) = stores.get_state(original_id).await.unwrap().unwrap();
    let mut state: ExecutionState =
        serde_json::from_str(&serde_json::to_string(&original).unwrap()).unwrap();
    let execution_id = original_id;
    state.transition_status(ExecutionStatus::Running).unwrap();
    state.set_node_state(
        node_key!("run"),
        nebula_execution::state::NodeExecutionState::new(),
    );
    let clock = Arc::new(CatalogClock {
        entry: Utc::now(),
        load_completed: std::sync::atomic::AtomicBool::new(false),
    });
    state.started_at = Some(clock.entry - chrono::Duration::seconds(1));
    state.budget.as_mut().unwrap().max_duration = Some(Duration::from_secs(10));
    stores.replace_exact_state(state).await;
    let catalog = Arc::new(ClockChangingCatalog {
        inner: Arc::new(stores.execution.plan_flavor_catalog()),
        clock: Arc::clone(&clock),
    });
    let (engine, _) = make_engine(runtime_registry);
    let engine = engine
        .with_execution_stores(stores.execution_stores())
        .with_clock(clock)
        .with_plan_flavor_runtime(
            Arc::new(crate::PlanFlavorRevisionLoader::new(catalog)),
            frozen,
            Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                &stores.execution,
            )),
        );
    let result = engine
        .resume_execution(&crate::store_seam::single_tenant_scope(), execution_id)
        .await
        .unwrap();
    assert_eq!(
        result.status,
        ExecutionStatus::Completed,
        "prior elapsed is sampled once at turn entry; catalog I/O belongs to the monotonic turn duration"
    );
    assert_eq!(instantiations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn corrupt_exact_plan_rejects_cold_warm_and_adopted_turns_without_dispatch() {
    for (warm, adopt) in [(false, false), (true, false), (true, true)] {
        let runtime_registry = Arc::new(ActionRegistry::new());
        let (frozen, instantiations) = snapshot_registry_counted(&runtime_registry, "must-not-run");
        let stores = TestStores::new();
        let (original_id, _) = install_snapshot_execution(&stores, &frozen).await;
        let (_, original) = stores.get_state(original_id).await.unwrap().unwrap();
        let mut state: ExecutionState =
            serde_json::from_str(&serde_json::to_string(&original).unwrap()).unwrap();
        let execution_id = original_id;
        if warm {
            state.transition_status(ExecutionStatus::Running).unwrap();
            state.set_node_state(
                node_key!("run"),
                nebula_execution::state::NodeExecutionState::new(),
            );
        }
        stores.replace_exact_state(state).await;
        let before = stores.get_state(execution_id).await.unwrap();
        let catalog = Arc::new(CorruptPlanCatalog {
            inner: Arc::new(stores.execution.plan_flavor_catalog()),
        });
        let (engine, _) = make_engine(runtime_registry);
        let engine = engine
            .with_execution_stores(stores.execution_stores())
            .with_plan_flavor_runtime(
                Arc::new(crate::PlanFlavorRevisionLoader::new(catalog)),
                frozen,
                Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                    &stores.execution,
                )),
            );
        let scope = crate::store_seam::single_tenant_scope();
        let error = if adopt {
            let fence = stores
                .execution
                .acquire_lease(
                    &scope,
                    &execution_id.to_string(),
                    "handoff",
                    Duration::from_secs(30),
                )
                .await
                .unwrap()
                .unwrap();
            engine
                .resume_execution_leased(&scope, execution_id, fence)
                .await
                .unwrap_err()
        } else {
            engine
                .resume_execution(&scope, execution_id)
                .await
                .unwrap_err()
        };
        assert!(matches!(error, EngineError::ExactRevision { .. }));
        assert_eq!(instantiations.load(Ordering::SeqCst), 0);
        assert!(!format!("{error:?} {error}").contains("secret-canary"));
        assert_eq!(stores.get_state(execution_id).await.unwrap(), before);
        assert!(
            stores
                .node_results
                .load_all_node_outputs(&scope, &execution_id.to_string())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            stores
                .execution
                .acquire_lease(
                    &scope,
                    &execution_id.to_string(),
                    "recovery",
                    Duration::from_secs(30)
                )
                .await
                .unwrap()
                .is_some()
        );
    }
}

#[tokio::test]
async fn durable_resume_rejects_pinned_state_without_materialized_contract() {
    let runtime_registry = Arc::new(ActionRegistry::new());
    let (frozen, instantiations) = snapshot_registry_counted(&runtime_registry, "must-not-run");
    let stores = TestStores::new();
    let (execution_id, _) = install_snapshot_execution_with_bundle(&stores, &frozen, false).await;
    let before = stores.get_state(execution_id).await.unwrap();
    let (engine, _) = make_engine(runtime_registry);
    let engine = engine
        .with_execution_stores(stores.execution_stores())
        .with_plan_flavor_runtime(
            Arc::new(crate::PlanFlavorRevisionLoader::new(Arc::new(
                stores.execution.plan_flavor_catalog(),
            ))),
            frozen,
            Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                &stores.execution,
            )),
        );
    let result = engine
        .resume_execution(&crate::store_seam::single_tenant_scope(), execution_id)
        .await;
    assert!(
        matches!(result, Err(EngineError::MissingContractBundle)),
        "revision pins alone must not authorize a durable turn without its bundle"
    );
    assert_eq!(instantiations.load(Ordering::SeqCst), 0);
    assert_eq!(stores.get_state(execution_id).await.unwrap(), before);
}

#[tokio::test]
async fn durable_resume_uses_stored_graph_and_retained_factory_after_mutable_replacement() {
    let runtime_registry = Arc::new(ActionRegistry::new());
    let frozen = snapshot_registry(&runtime_registry, "original-A");
    let stores = TestStores::new();
    let (execution_id, mut workflow) = install_snapshot_execution(&stores, &frozen).await;
    // Publishing a different graph and swapping the same action/version in the
    // mutable registry must affect neither the durable graph nor its factory.
    workflow.nodes[0].id = node_key!("replacement-node");
    stores.save_workflow(&workflow).await;
    let _replacement = snapshot_registry(&runtime_registry, "replacement-B");
    let (engine, _) = make_engine(runtime_registry);
    let engine = engine
        .with_execution_stores(stores.execution_stores())
        .with_plan_flavor_runtime(
            Arc::new(crate::PlanFlavorRevisionLoader::new(Arc::new(
                stores.execution.plan_flavor_catalog(),
            ))),
            frozen,
            Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                &stores.execution,
            )),
        );
    let result = engine
        .resume_execution(&crate::store_seam::single_tenant_scope(), execution_id)
        .await
        .unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed);
    assert_eq!(
        result.node_output(&node_key!("run")),
        Some(&serde_json::json!("original-A"))
    );
    assert_eq!(result.node_output(&node_key!("replacement-node")), None);
    let (_, persisted) = stores.get_state(execution_id).await.unwrap().unwrap();
    assert_eq!(
        persisted["total_output_bytes"],
        serde_json::json!(
            serde_json::to_vec(&serde_json::json!("original-A"))
                .unwrap()
                .len()
        ),
        "the output budget counter must survive the final durable checkpoint"
    );
}

#[tokio::test]
async fn durable_resume_missing_runtime_preserves_state_and_releases_adopted_fence() {
    let runtime_registry = Arc::new(ActionRegistry::new());
    let (frozen, instantiations) = snapshot_registry_counted(&runtime_registry, "must-not-run");
    let stores = TestStores::new();
    let (execution_id, _) = install_snapshot_execution(&stores, &frozen).await;
    let (engine, _) = make_engine(runtime_registry);
    let engine = engine.with_execution_stores(stores.execution_stores());
    let scope = crate::store_seam::single_tenant_scope();
    let before = stores.get_state(execution_id).await.unwrap();
    let fence = stores
        .execution
        .acquire_lease(
            &scope,
            &execution_id.to_string(),
            "handoff",
            Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap();
    let error = engine
        .resume_execution_leased(&scope, execution_id, fence)
        .await
        .unwrap_err();
    assert!(matches!(error, EngineError::MissingExactRuntime));
    assert_eq!(instantiations.load(Ordering::SeqCst), 0);
    assert_eq!(stores.get_state(execution_id).await.unwrap(), before);
    assert!(
        stores
            .execution
            .acquire_lease(
                &scope,
                &execution_id.to_string(),
                "correct-worker",
                Duration::from_secs(30)
            )
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        stores
            .node_results
            .load_all_node_outputs(&scope, &execution_id.to_string())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn durable_resume_does_not_restart_persisted_duration_budget() {
    let runtime_registry = Arc::new(ActionRegistry::new());
    let frozen = snapshot_registry(&runtime_registry, "must-not-run");
    let stores = TestStores::new();
    let (original_id, _) = install_snapshot_execution(&stores, &frozen).await;
    let (_, original) = stores.get_state(original_id).await.unwrap().unwrap();
    let mut state: ExecutionState =
        serde_json::from_str(&serde_json::to_string(&original).unwrap()).unwrap();
    let execution_id = original_id;
    state.started_at = Some(Utc::now() - chrono::Duration::seconds(10));
    state.budget.as_mut().unwrap().max_duration = Some(Duration::from_secs(1));
    stores.replace_exact_state(state).await;
    let (engine, _) = make_engine(runtime_registry);
    let engine = engine
        .with_execution_stores(stores.execution_stores())
        .with_plan_flavor_runtime(
            Arc::new(crate::PlanFlavorRevisionLoader::new(Arc::new(
                stores.execution.plan_flavor_catalog(),
            ))),
            frozen,
            Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                &stores.execution,
            )),
        );
    let result = engine
        .resume_execution(&crate::store_seam::single_tenant_scope(), execution_id)
        .await
        .unwrap();
    assert!(
        !result.is_success(),
        "a resumed turn must retain the elapsed budget"
    );
    assert_eq!(result.node_output(&node_key!("run")), None);
}

#[tokio::test]
async fn durable_resume_rejects_unpinned_state_before_action_dispatch() {
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(
            ActionMetadataDraft::new(
                action_key!("echo"),
                nebula_action::metadata_name!("Echo"),
                "echoes input",
            ),
            EchoHandler,
        )
        .expect("valid test catalog definition");
    let (engine, _) = make_engine(registry);
    let stores = TestStores::new();
    let workflow = make_workflow(
        vec![NodeDefinition::new(node_key!("run"), "Run", "core", "echo").unwrap()],
        vec![],
    );
    stores.save_workflow(&workflow).await;
    let execution_id = ExecutionId::new();
    let mut state = ExecutionState::new(execution_id, workflow.id, &[]);
    state.set_budget(ExecutionBudget::default());
    stores
        .inject_state(
            execution_id,
            workflow.id,
            serde_json::to_value(&state).unwrap(),
        )
        .await;
    let result = stores
        .attach(engine)
        .resume_execution(&crate::store_seam::single_tenant_scope(), execution_id)
        .await;
    assert!(
        result.is_err(),
        "unpinned durable execution must not dispatch: {result:?}"
    );
    let (_, persisted) = stores.get_state(execution_id).await.unwrap().unwrap();
    assert_eq!(
        persisted["status"],
        serde_json::to_value(ExecutionStatus::Created).unwrap()
    );
}
