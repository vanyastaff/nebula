use super::*;

struct DriftingFactory {
    original: ActionMetadata,
    changed: ActionMetadata,
    drifted: Arc<std::sync::atomic::AtomicBool>,
    instantiations: Arc<AtomicU32>,
    inner: Arc<dyn nebula_action::ActionFactory>,
}
impl nebula_action::ActionFactory for DriftingFactory {
    fn metadata(&self) -> &ActionMetadata {
        if self.drifted.load(Ordering::SeqCst) {
            &self.changed
        } else {
            &self.original
        }
    }
    fn dependencies(&self) -> &Dependencies {
        self.inner.dependencies()
    }
    fn instantiate<'a>(
        &'a self,
        node: &'a NodeDefinition,
        context: &'a dyn nebula_action::ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<nebula_action::ActionHandle, ActionError>> + Send + 'a>>
    {
        self.instantiations.fetch_add(1, Ordering::SeqCst);
        self.inner.instantiate(node, context)
    }
}

#[tokio::test]
async fn exact_turn_rejects_factory_effect_or_version_drift_before_instantiation() {
    for change_version in [false, true] {
        let runtime_registry = Arc::new(ActionRegistry::new());
        let metadata = SnapshotHandler::metadata()
            .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
            .with_schema(nebula_schema::ValidSchema::empty())
            .with_output_schema(nebula_schema::ValidSchema::empty());
        runtime_registry
            .register_stateless_instance(metadata.clone(), SnapshotHandler("must-not-run"));
        let inner = runtime_registry
            .get_factory(&action_key!("exact.run"))
            .unwrap()
            .1;
        let mut changed = metadata.clone();
        if change_version {
            changed.base.version = semver::Version::new(99, 0, 0);
        } else {
            changed.effect_contract = nebula_action::effect::ActionEffectContract::Undeclared;
        }
        let drifted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let instantiations = Arc::new(AtomicU32::new(0));
        let plugin = SnapshotPlugin {
            manifest: nebula_plugin::PluginManifest::builder("exact", "Exact")
                .build()
                .unwrap(),
            factory: Arc::new(DriftingFactory {
                original: metadata,
                changed,
                drifted: drifted.clone(),
                instantiations: instantiations.clone(),
                inner,
            }),
        };
        let mut registry = PluginRegistry::new();
        registry
            .register(Arc::new(
                nebula_plugin::ResolvedPlugin::from(plugin).unwrap(),
            ))
            .unwrap();
        let frozen = Arc::new(
            registry
                .freeze(
                    nebula_core::ArtifactSetDigest::from_bytes([0x73; 32]),
                    "1.0.0".parse().unwrap(),
                )
                .unwrap(),
        );
        let stores = TestStores::new();
        let (id, _) = install_snapshot_execution(&stores, &frozen).await;
        let before = stores.get_state(id).await.unwrap();
        let (engine, _) = make_engine(runtime_registry);
        let engine = stores.attach(engine).with_plan_flavor_runtime(
            Arc::new(crate::PlanFlavorRevisionLoader::new(Arc::new(
                stores.execution.plan_flavor_catalog(),
            ))),
            frozen,
            Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                &stores.execution,
            )),
        );
        drifted.store(true, Ordering::SeqCst);
        let error = engine
            .resume_execution(&crate::store_seam::single_tenant_scope(), id)
            .await
            .unwrap_err();
        assert!(matches!(error, EngineError::ExactFactoryUnavailable));
        assert_eq!(instantiations.load(Ordering::SeqCst), 0);
        assert_eq!(stores.get_state(id).await.unwrap(), before);
    }
}

#[tokio::test]
async fn malformed_non_action_checkpoint_facts_reject_before_instantiation() {
    for kind in ["recovered", "bypassed", "skipped"] {
        let runtime_registry = Arc::new(ActionRegistry::new());
        let (frozen, instantiations) = snapshot_registry_counted(&runtime_registry, "must-not-run");
        let stores = TestStores::new();
        let (id, _) = install_snapshot_execution(&stores, &frozen).await;
        let scope = crate::store_seam::single_tenant_scope();
        let (version, mut state) = stores.get_state(id).await.unwrap().unwrap();
        state["checkpoint"]["nodes"]["run"] =
            serde_json::json!({"kind":kind,"unexpected":"secret-canary"});
        let fence = stores
            .execution
            .acquire_lease(
                &scope,
                &id.to_string(),
                "corrupt-checkpoint-fixture",
                Duration::from_secs(30),
            )
            .await
            .unwrap()
            .unwrap();
        let outcome = stores
            .execution
            .commit(
                nebula_storage_port::TransitionBatch::builder()
                    .scope(scope.clone())
                    .execution_id(id.to_string())
                    .expected_version(version)
                    .fencing(fence)
                    .new_state(state)
                    .build()
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            nebula_storage_port::TransitionOutcome::Applied { .. }
        ));
        stores
            .execution
            .release_lease(&scope, &id.to_string(), fence)
            .await
            .unwrap();
        let before = stores.get_state(id).await.unwrap();
        let (engine, _) = make_engine(runtime_registry);
        let engine = stores.attach(engine).with_plan_flavor_runtime(
            Arc::new(crate::PlanFlavorRevisionLoader::new(Arc::new(
                stores.execution.plan_flavor_catalog(),
            ))),
            frozen,
            Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                &stores.execution,
            )),
        );
        let error = engine.resume_execution(&scope, id).await.unwrap_err();
        assert!(matches!(error, EngineError::InvalidRecordedExecution));
        assert!(!format!("{error:?} {error}").contains("secret-canary"));
        assert_eq!(instantiations.load(Ordering::SeqCst), 0);
        assert_eq!(stores.get_state(id).await.unwrap(), before);
    }
}

fn completed(state: &mut ExecutionState, node: &NodeKey, result: &ActionResult<serde_json::Value>) {
    state.start_node_attempt(node.clone()).unwrap();
    state
        .transition_node(node.clone(), NodeState::Completed)
        .unwrap();
    state
        .checkpoint
        .as_mut()
        .unwrap()
        .insert(node.clone(), checkpoint::action_checkpoint(result).unwrap());
}

#[test]
fn checkpoint_limit_counts_unselected_and_non_primary_payloads() {
    let node = node_key!("node");
    let mut state = ExecutionState::new(
        ExecutionId::new(),
        WorkflowId::new(),
        std::slice::from_ref(&node),
    );
    state.set_budget(ExecutionBudget::default().with_max_output_bytes(128));
    let alternatives = ActionResult::Branch {
        selected: BranchKey::new("selected").unwrap(),
        output: nebula_action::ActionOutput::Value(serde_json::Value::Null),
        alternatives: HashMap::from([(
            BranchKey::new("other").unwrap(),
            nebula_action::ActionOutput::Value(serde_json::json!("x".repeat(1024))),
        )]),
    };
    completed(&mut state, &node, &alternatives);
    let nodes = HashSet::from([node.clone()]);
    assert!(matches!(
        checkpoint::validated_checkpoint_outputs(&state, &nodes, &HashSet::new()),
        Err(EngineError::CheckpointPayloadLimit)
    ));
    state.checkpoint.as_mut().unwrap().insert(
        node,
        checkpoint::action_checkpoint(&ActionResult::MultiOutput {
            outputs: HashMap::from([(
                port_key!("other"),
                nebula_action::ActionOutput::Value(serde_json::json!("x".repeat(1024))),
            )]),
            main_output: None,
        })
        .unwrap(),
    );
    assert!(matches!(
        checkpoint::validated_checkpoint_outputs(&state, &nodes, &HashSet::new()),
        Err(EngineError::CheckpointPayloadLimit)
    ));
}

#[test]
fn default_budget_still_enforces_the_durable_checkpoint_ceiling() {
    let node = node_key!("node");
    let mut state = ExecutionState::new(
        ExecutionId::new(),
        WorkflowId::new(),
        std::slice::from_ref(&node),
    );
    state.node_states.get_mut(&node).unwrap().current_output =
        Some(nebula_execution::NodeOutput::inline(
            serde_json::json!(
                "x".repeat(usize::try_from(checkpoint::MAX_DURABLE_CHECKPOINT_BYTES).unwrap())
            ),
            NodeState::Completed,
            checkpoint::MAX_DURABLE_CHECKPOINT_BYTES,
        ));

    std::assert_matches!(
        checkpoint::validate_checkpoint_size(&state),
        Err(EngineError::CheckpointPayloadLimit)
    );
}

#[test]
fn encoded_checkpoint_is_bounded_before_execution_state_decode() {
    let encoded_state = serde_json::json!({
        "checkpoint": {
            "format_version": 1,
            "nodes": {
                "node": {
                    "kind": "action_result",
                    "format_version": 1,
                    "value": "x".repeat(
                        usize::try_from(checkpoint::MAX_DURABLE_CHECKPOINT_BYTES).unwrap()
                    )
                }
            }
        },
        "node_states": {}
    });

    std::assert_matches!(
        checkpoint::validate_encoded_checkpoint_size(&encoded_state),
        Err(EngineError::CheckpointPayloadLimit)
    );
}

#[test]
fn checkpoint_requires_supported_format_and_terminal_evidence() {
    let node = node_key!("node");
    let mut state = ExecutionState::new(
        ExecutionId::new(),
        WorkflowId::new(),
        std::slice::from_ref(&node),
    );
    state.start_node_attempt(node.clone()).unwrap();
    state
        .transition_node(node.clone(), NodeState::Completed)
        .unwrap();
    let nodes = HashSet::from([node.clone()]);
    assert!(matches!(
        checkpoint::validated_checkpoint_outputs(&state, &nodes, &HashSet::new()),
        Err(EngineError::InvalidRecordedCheckpoint)
    ));
    state.checkpoint.as_mut().unwrap().insert(
        node,
        nebula_execution::NodeCheckpoint::ActionResult {
            format_version: 99,
            value: serde_json::json!({"secret": "must remain redacted"}),
        },
    );
    let error =
        checkpoint::validated_checkpoint_outputs(&state, &nodes, &HashSet::new()).unwrap_err();
    assert!(matches!(error, EngineError::InvalidRecordedCheckpoint));
    assert!(!format!("{error:?} {error}").contains("must remain redacted"));
}

#[tokio::test]
async fn resume_does_not_activate_an_all_dead_join() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echo"),
        EchoHandler,
    );
    let stores = TestStores::new();
    let (engine, _) = make_engine(registry);
    let a = node_key!("a");
    let b = node_key!("b");
    let join = node_key!("join");
    let workflow = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "core", "echo").unwrap(),
            NodeDefinition::new(b.clone(), "B", "core", "echo").unwrap(),
            NodeDefinition::new(join.clone(), "Join", "core", "echo").unwrap(),
        ],
        vec![
            Connection::new(a.clone(), join.clone()),
            Connection::new(b.clone(), join.clone()),
        ],
    );
    let id = ExecutionId::new();
    let mut state = ExecutionState::new(id, workflow.id, &[a.clone(), b.clone(), join.clone()]);
    state.transition_status(ExecutionStatus::Running).unwrap();
    completed(&mut state, &a, &ActionResult::skip("dead branch A"));
    completed(&mut state, &b, &ActionResult::skip("dead branch B"));
    let engine = stores
        .attach_exact(engine, &workflow, serde_json::to_value(state).unwrap())
        .await;
    let result = engine
        .resume_execution(&crate::store_seam::single_tenant_scope(), id)
        .await
        .unwrap();
    assert!(result.is_success());
    assert!(result.node_output(&join).is_none());
    let (_, saved) = stores.get_state(id).await.unwrap().unwrap();
    assert_eq!(saved["node_states"][join.as_str()]["state"], "skipped");
}

#[tokio::test]
async fn resume_counts_distinct_ports_from_same_source_once_each() {
    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ActionMetadata::new(action_key!("echo"), "Echo", "echo"),
        EchoHandler,
    );
    let stores = TestStores::new();
    let (engine, _) = make_engine(registry);
    let a = node_key!("a");
    let b = node_key!("b");
    let workflow = make_workflow(
        vec![
            NodeDefinition::new(a.clone(), "A", "core", "echo").unwrap(),
            NodeDefinition::new(b.clone(), "B", "core", "echo").unwrap(),
        ],
        vec![
            Connection::new(a.clone(), b.clone()),
            Connection::new(a.clone(), b.clone()).with_from_port(port_key!("error")),
        ],
    );
    let id = ExecutionId::new();
    let mut state = ExecutionState::new(id, workflow.id, &[a.clone(), b.clone()]);
    state.transition_status(ExecutionStatus::Running).unwrap();
    completed(
        &mut state,
        &a,
        &ActionResult::success(serde_json::json!("committed")),
    );
    let engine = stores
        .attach_exact(engine, &workflow, serde_json::to_value(state).unwrap())
        .await;
    let result = engine
        .resume_execution(&crate::store_seam::single_tenant_scope(), id)
        .await
        .unwrap();
    assert!(result.is_success());
    assert_eq!(
        result.node_output(&b),
        Some(&serde_json::json!("committed"))
    );
}

#[tokio::test]
async fn checkpoint_commits_only_processed_node_and_replaces_partial_output() {
    let stores = TestStores::new();
    let (engine, _) = make_engine(Arc::new(ActionRegistry::new()));
    let engine = stores.attach(engine);
    let scope = crate::store_seam::single_tenant_scope();
    let a = node_key!("a");
    let b = node_key!("b");
    let id = ExecutionId::new();
    let mut state = ExecutionState::new(id, WorkflowId::new(), &[a.clone(), b.clone()]);
    state.transition_status(ExecutionStatus::Running).unwrap();
    state.start_node_attempt(a.clone()).unwrap();
    state.start_node_attempt(b.clone()).unwrap();
    stores
        .inject_state(id, state.workflow_id, serde_json::to_value(&state).unwrap())
        .await;
    let fence = stores
        .execution
        .acquire_lease(
            &scope,
            &id.to_string(),
            "checkpoint-test",
            Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap();
    let outputs = Arc::new(DashMap::new());
    outputs.insert(a.clone(), serde_json::json!("partial"));
    outputs.insert(b.clone(), serde_json::json!("unprocessed sibling secret"));
    let mut version = 0;
    state
        .override_node_state(a.clone(), NodeState::Waiting)
        .unwrap();
    let wait = ActionResult::Wait {
        condition: WaitCondition::Duration {
            duration: Duration::from_secs(1),
        },
        timeout: None,
        partial_output: Some(nebula_action::ActionOutput::Value(serde_json::json!(
            "partial"
        ))),
    };
    engine
        .checkpoint_node(
            &scope,
            id,
            a.clone(),
            Some(checkpoint::action_checkpoint(&wait).unwrap()),
            &outputs,
            &mut state,
            &mut version,
            Some(fence),
            vec![],
        )
        .await
        .unwrap();
    let (_, first) = stores.get_state(id).await.unwrap().unwrap();
    assert!(first["checkpoint"]["nodes"].get(b.as_str()).is_none());
    assert!(!first.to_string().contains("unprocessed sibling secret"));
    state
        .transition_node(a.clone(), NodeState::Completed)
        .unwrap();
    engine
        .checkpoint_node(
            &scope,
            id,
            a.clone(),
            Some(checkpoint::action_checkpoint(&ActionResult::skip("outputless")).unwrap()),
            &outputs,
            &mut state,
            &mut version,
            Some(fence),
            vec![],
        )
        .await
        .unwrap();
    let evidence = state.checkpoint.as_ref().unwrap().nodes().get(&a).unwrap();
    assert!(checkpoint::checkpoint_output(evidence).unwrap().is_none());
    let (_, second) = stores.get_state(id).await.unwrap().unwrap();
    assert!(!second["checkpoint"].to_string().contains("partial"));
    stores
        .execution
        .release_lease(&scope, &id.to_string(), fence)
        .await
        .unwrap();
}

#[tokio::test]
async fn failed_checkpoint_does_not_install_candidate_for_a_later_write() {
    let stores = TestStores::new();
    let failing = Arc::new(FailAtCommitN::new(stores.execution.clone(), 1));
    let (engine, _) = make_engine(Arc::new(ActionRegistry::new()));
    let engine = engine.with_execution_stores(crate::store_seam::ExecutionStores {
        execution: failing,
        ..stores.execution_stores()
    });
    let scope = crate::store_seam::single_tenant_scope();
    let node = node_key!("node");
    let id = ExecutionId::new();
    let mut state = ExecutionState::new(id, WorkflowId::new(), std::slice::from_ref(&node));
    state.transition_status(ExecutionStatus::Running).unwrap();
    state.start_node_attempt(node.clone()).unwrap();
    stores
        .inject_state(id, state.workflow_id, serde_json::to_value(&state).unwrap())
        .await;
    let fence = stores
        .execution
        .acquire_lease(
            &scope,
            &id.to_string(),
            "checkpoint-test",
            Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap();
    let outputs = Arc::new(DashMap::new());
    let mut version = 0;
    let error = engine
        .checkpoint_node(
            &scope,
            id,
            node.clone(),
            Some(
                checkpoint::action_checkpoint(&ActionResult::success(serde_json::json!(
                    "uncommitted"
                )))
                .unwrap(),
            ),
            &outputs,
            &mut state,
            &mut version,
            Some(fence),
            vec![],
        )
        .await
        .unwrap_err();
    assert!(matches!(error, EngineError::CheckpointFailed { .. }));
    assert!(
        !state
            .checkpoint
            .as_ref()
            .unwrap()
            .nodes()
            .contains_key(&node)
    );
    // A later unrelated owner write must still contain only committed evidence.
    engine
        .checkpoint_node(
            &scope,
            id,
            node.clone(),
            None,
            &outputs,
            &mut state,
            &mut version,
            Some(fence),
            vec![],
        )
        .await
        .unwrap();
    let (_, saved) = stores.get_state(id).await.unwrap().unwrap();
    assert!(!saved.to_string().contains("uncommitted"));
    stores
        .execution
        .release_lease(&scope, &id.to_string(), fence)
        .await
        .unwrap();
}
