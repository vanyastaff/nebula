//! Integration tests for [`ControlAction`] DX family.
//!
//! These tests act as runnable documentation for community plugin authors:
//! each of the seven canonical control-flow node types (`If`, `Switch`,
//! `Router`, `Filter`, `NoOp`, `Stop`, `Fail`) is implemented here as a
//! **test fixture** — not production code — to demonstrate how the trait
//! contract supports each semantic.
//!
//! The fixtures use only the public `nebula_action::*` surface, so this
//! file doubles as a compile-time check that everything a community author
//! needs is actually re-exported.
//!
//! The canonical production implementations of these nodes will live in a
//! downstream crate (placement TBD) and are intentionally not created here —
//! the goal of this test is to validate the trait/adapter infrastructure,
//! not to ship batteries-included nodes.

use std::sync::{Arc, OnceLock};

use nebula_action::{
    Action, ActionError, ActionInput, ActionKind, ActionOutput, ActionResult, ActionRuntimeContext,
    BranchKey, ControlAction, ControlActionAdapter, ControlHandle, ControlOutcome, OutputPort,
    TerminationReason, ValidationReason, port_key, testing::TestContextBuilder,
};
use nebula_core::{Dependencies, action_key};
use nebula_schema::{FieldCollector, HasSchema, Schema, StringBuilder, ValidSchema, field_key};

// ── Test helpers ───────────────────────────────────────────────────────────

fn make_ctx() -> ActionRuntimeContext {
    TestContextBuilder::new().build()
}

async fn run(
    adapter: &impl ControlHandle,
    input: serde_json::Value,
) -> ActionResult<serde_json::Value> {
    let ctx = make_ctx();
    let input = adapter
        .prepare_input(ActionInput::Raw(input))
        .expect("input should prepare");
    adapter
        .dispatch(input, &ctx)
        .await
        .expect("execute should succeed")
}

async fn run_err(adapter: &impl ControlHandle, input: serde_json::Value) -> ActionError {
    let ctx = make_ctx();
    match adapter.prepare_input(ActionInput::Raw(input)) {
        Ok(input) => adapter
            .dispatch(input, &ctx)
            .await
            .expect_err("execute should fail"),
        Err(error) => error,
    }
}

fn required_bool(input: &serde_json::Value, pointer: &str) -> Result<bool, ActionError> {
    input
        .pointer(pointer)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| {
            ActionError::validation(
                "control_input",
                ValidationReason::WrongType,
                Some(format!("expected boolean at `{pointer}`")),
            )
        })
}

fn required_str<'a>(input: &'a serde_json::Value, pointer: &str) -> Result<&'a str, ActionError> {
    input
        .pointer(pointer)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            ActionError::validation(
                "control_input",
                ValidationReason::WrongType,
                Some(format!("expected string at `{pointer}`")),
            )
        })
}

fn required_i64(input: &serde_json::Value, pointer: &str) -> Result<i64, ActionError> {
    input
        .pointer(pointer)
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| {
            ActionError::validation(
                "control_input",
                ValidationReason::WrongType,
                Some(format!("expected i64 at `{pointer}`")),
            )
        })
}

// ── DemoIf ─ binary branch ─────────────────────────────────────────────────

struct DemoIf;

#[derive(serde::Deserialize, serde::Serialize, nebula_schema::Schema)]
struct DemoIfInput {
    condition: bool,
    value: Option<i64>,
}

impl Action for DemoIf {
    type Input = DemoIfInput;
    type Output = DemoIfInput;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("demo.if"),
            nebula_action::metadata_name!("If"),
            "Binary branch",
        )
        .with_outputs(vec![
            OutputPort::flow(port_key!("true")),
            OutputPort::flow(port_key!("false")),
        ])
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl ControlAction for DemoIf {
    async fn evaluate(
        &self,
        input: DemoIfInput,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ControlOutcome<DemoIfInput>, ActionError> {
        let selected = if input.condition {
            BranchKey::new("true")
        } else {
            BranchKey::new("false")
        }
        .expect("'true'/'false' are valid branch key literals");
        Ok(ControlOutcome::Branch {
            selected,
            output: input,
        })
    }
}

#[tokio::test]
async fn demo_if_routes_true() {
    let adapter = ControlActionAdapter::new(DemoIf).expect("valid test catalog definition");
    let result = run(
        &adapter,
        serde_json::json!({ "condition": true, "value": 42 }),
    )
    .await;
    match result {
        ActionResult::Branch {
            selected, output, ..
        } => {
            assert_eq!(selected.as_str(), "true");
            assert_eq!(
                output.as_value(),
                Some(&serde_json::json!({ "condition": true, "value": 42 }))
            );
        },
        _ => panic!("expected Branch"),
    }
}

#[tokio::test]
async fn demo_if_routes_false() {
    let adapter = ControlActionAdapter::new(DemoIf).expect("valid test catalog definition");
    let result = run(&adapter, serde_json::json!({ "condition": false })).await;
    match result {
        ActionResult::Branch { selected, .. } => assert_eq!(selected.as_str(), "false"),
        _ => panic!("expected Branch"),
    }
}

#[tokio::test]
async fn demo_if_missing_condition_is_validation_error() {
    let adapter = ControlActionAdapter::new(DemoIf).expect("valid test catalog definition");
    let err = run_err(&adapter, serde_json::json!({})).await;
    match err {
        ActionError::Validation { reason, .. } => {
            assert_eq!(reason, ValidationReason::MissingField);
        },
        _ => panic!("expected Validation error"),
    }
}

#[tokio::test]
async fn demo_if_wrong_type_is_validation_error() {
    let adapter = ControlActionAdapter::new(DemoIf).expect("valid test catalog definition");
    let err = run_err(&adapter, serde_json::json!({ "condition": "yes" })).await;
    match err {
        ActionError::Validation { reason, .. } => {
            assert_eq!(reason, ValidationReason::WrongType);
        },
        _ => panic!("expected Validation error"),
    }
}

#[test]
fn demo_if_has_control_kind() {
    let adapter = ControlActionAdapter::new(DemoIf).expect("valid test catalog definition");
    assert_eq!(adapter.metadata().kind(), ActionKind::Control);
}

// ── DemoSwitch ─ N-way static branch ───────────────────────────────────────

struct DemoSwitch;

impl Action for DemoSwitch {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("demo.switch"),
            nebula_action::metadata_name!("Switch"),
            "N-way branch by status field",
        )
        .with_outputs(vec![
            OutputPort::flow(port_key!("active")),
            OutputPort::flow(port_key!("pending")),
            OutputPort::flow(port_key!("archived")),
            OutputPort::flow(port_key!("default")),
        ])
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl ControlAction for DemoSwitch {
    async fn evaluate(
        &self,
        input: serde_json::Value,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ControlOutcome<serde_json::Value>, ActionError> {
        let status = required_str(&input, "/status")?;
        let branch_name = match status {
            "active" | "pending" | "archived" => status,
            _ => "default",
        };
        let selected =
            BranchKey::new(branch_name).expect("switch branch names are valid key literals");
        Ok(ControlOutcome::Branch {
            selected,
            output: input,
        })
    }
}

#[tokio::test]
async fn demo_switch_routes_known_case() {
    let adapter = ControlActionAdapter::new(DemoSwitch).expect("valid test catalog definition");
    let result = run(&adapter, serde_json::json!({ "status": "pending" })).await;
    match result {
        ActionResult::Branch { selected, .. } => assert_eq!(selected.as_str(), "pending"),
        _ => panic!("expected Branch"),
    }
}

#[tokio::test]
async fn demo_switch_falls_back_to_default() {
    let adapter = ControlActionAdapter::new(DemoSwitch).expect("valid test catalog definition");
    let result = run(&adapter, serde_json::json!({ "status": "unknown" })).await;
    match result {
        ActionResult::Branch { selected, .. } => assert_eq!(selected.as_str(), "default"),
        _ => panic!("expected Branch"),
    }
}

// ── DemoRouter ─ multi-match routing ───────────────────────────────────────

#[derive(Clone, Copy)]
enum RouterMode {
    FirstMatch,
    AllMatch,
}

struct DemoRouter {
    mode: RouterMode,
}

impl DemoRouter {
    fn new(mode: RouterMode) -> Self {
        Self { mode }
    }

    fn classify(priority: i64) -> Vec<&'static str> {
        let mut out = Vec::new();
        if priority >= 100 {
            out.push("high");
        }
        if (10..=500).contains(&priority) {
            out.push("medium");
        }
        if priority < 50 {
            out.push("low");
        }
        out
    }
}

impl Action for DemoRouter {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("demo.router"),
            nebula_action::metadata_name!("Router"),
            "Multi-rule routing",
        )
        .with_outputs(vec![
            OutputPort::flow(port_key!("high")),
            OutputPort::flow(port_key!("medium")),
            OutputPort::flow(port_key!("low")),
        ])
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl ControlAction for DemoRouter {
    async fn evaluate(
        &self,
        input: serde_json::Value,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ControlOutcome<serde_json::Value>, ActionError> {
        let priority = required_i64(&input, "/priority")?;
        let matches = Self::classify(priority);
        match (self.mode, matches.as_slice()) {
            (_, []) => Ok(ControlOutcome::Drop {
                reason: Some(format!("no rule matched priority {priority}")),
            }),
            (RouterMode::FirstMatch, _) => {
                let value = input;
                let selected =
                    BranchKey::new(matches[0]).expect("router branch names are valid key literals");
                Ok(ControlOutcome::Branch {
                    selected,
                    output: value,
                })
            },
            (RouterMode::AllMatch, _) => {
                let value = input;
                let ports: std::collections::HashMap<_, _> = matches
                    .iter()
                    .map(|port_name| {
                        let key = nebula_action::PortKey::new(*port_name)
                            .expect("router port names are valid key literals");
                        (key, value.clone())
                    })
                    .collect();
                Ok(ControlOutcome::Route { ports })
            },
        }
    }
}

#[tokio::test]
async fn demo_router_first_match_picks_first_rule() {
    let adapter = ControlActionAdapter::new(DemoRouter::new(RouterMode::FirstMatch))
        .expect("valid test catalog definition");
    let result = run(&adapter, serde_json::json!({ "priority": 150 })).await;
    match result {
        ActionResult::Branch { selected, .. } => assert_eq!(selected.as_str(), "high"),
        _ => panic!("expected Branch"),
    }
}

#[tokio::test]
async fn demo_router_all_match_fires_multiple_ports() {
    let adapter = ControlActionAdapter::new(DemoRouter::new(RouterMode::AllMatch))
        .expect("valid test catalog definition");
    // priority=150 matches `high` (>=100) AND `medium` (10..=500)
    let result = run(&adapter, serde_json::json!({ "priority": 150 })).await;
    match result {
        ActionResult::MultiOutput {
            outputs,
            main_output,
        } => {
            assert_eq!(outputs.len(), 2);
            assert!(outputs.contains_key("high"));
            assert!(outputs.contains_key("medium"));
            assert!(main_output.is_none());
        },
        _ => panic!("expected MultiOutput"),
    }
}

/// Every i64 priority hits at least one of `DemoRouter`'s three classify
/// ranges, so an independent fixture is required to exercise the
/// `ControlOutcome::Drop` code path on a router-shaped action.
struct NeverMatchRouter;

impl Action for NeverMatchRouter {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("demo.never_match_router"),
            nebula_action::metadata_name!("NeverMatchRouter"),
            "Drops every input — used to test Drop code path",
        )
        .with_outputs(vec![OutputPort::flow(port_key!("out"))])
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl ControlAction for NeverMatchRouter {
    async fn evaluate(
        &self,
        _input: serde_json::Value,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ControlOutcome<serde_json::Value>, ActionError> {
        Ok(ControlOutcome::Drop {
            reason: Some("sentinel".into()),
        })
    }
}

#[tokio::test]
async fn demo_router_no_rules_match_drops() {
    let adapter =
        ControlActionAdapter::new(NeverMatchRouter).expect("valid test catalog definition");
    let result = run(&adapter, serde_json::json!({ "priority": 999 })).await;
    match result {
        ActionResult::Drop { reason } => assert_eq!(reason.as_deref(), Some("sentinel")),
        _ => panic!("expected Drop"),
    }
}

// ── DemoFilter ─ drop-or-pass gate ─────────────────────────────────────────

struct DemoFilter;

impl Action for DemoFilter {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("demo.filter"),
            nebula_action::metadata_name!("Filter"),
            "Drop items below threshold",
        )
        .with_outputs(vec![OutputPort::flow(port_key!("out"))])
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl ControlAction for DemoFilter {
    async fn evaluate(
        &self,
        input: serde_json::Value,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ControlOutcome<serde_json::Value>, ActionError> {
        let score = required_i64(&input, "/score")?;
        if score >= 50 {
            Ok(ControlOutcome::Pass { output: input })
        } else {
            Ok(ControlOutcome::Drop {
                reason: Some(format!("score {score} below 50")),
            })
        }
    }
}

#[tokio::test]
async fn demo_filter_passes_above_threshold() {
    let adapter = ControlActionAdapter::new(DemoFilter).expect("valid test catalog definition");
    let result = run(&adapter, serde_json::json!({ "score": 75 })).await;
    match result {
        ActionResult::Success { output } => {
            assert_eq!(output.as_value(), Some(&serde_json::json!({ "score": 75 })));
        },
        _ => panic!("expected Success"),
    }
}

#[tokio::test]
async fn demo_filter_drops_below_threshold() {
    let adapter = ControlActionAdapter::new(DemoFilter).expect("valid test catalog definition");
    let result = run(&adapter, serde_json::json!({ "score": 10 })).await;
    match result {
        ActionResult::Drop { reason } => {
            assert_eq!(reason.as_deref(), Some("score 10 below 50"));
        },
        _ => panic!("expected Drop"),
    }
}

#[tokio::test]
async fn demo_filter_distinguishes_drop_from_skip() {
    // Drop is semantically different from Skip. This test documents that
    // a ControlAction author must use Drop for "this item failed the
    // predicate" — not Skip, which would cancel the downstream subgraph.
    let adapter = ControlActionAdapter::new(DemoFilter).expect("valid test catalog definition");
    let result = run(&adapter, serde_json::json!({ "score": 5 })).await;
    assert!(result.is_drop(), "Filter must use Drop, not Skip");
    assert!(!matches!(result, ActionResult::Skip { .. }));
}

// ── DemoNoOp ─ pure passthrough ────────────────────────────────────────────

struct DemoNoOp;

impl Action for DemoNoOp {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("demo.noop"),
            nebula_action::metadata_name!("NoOp"),
            "Pass-through placeholder",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl ControlAction for DemoNoOp {
    async fn evaluate(
        &self,
        input: serde_json::Value,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ControlOutcome<serde_json::Value>, ActionError> {
        Ok(ControlOutcome::Pass { output: input })
    }
}

#[tokio::test]
async fn demo_noop_preserves_input() {
    let adapter = ControlActionAdapter::new(DemoNoOp).expect("valid test catalog definition");
    let input = serde_json::json!({ "arbitrary": { "nested": [1, 2, 3] } });
    let result = run(&adapter, input.clone()).await;
    match result {
        ActionResult::Success { output } => {
            assert_eq!(output.as_value(), Some(&input));
        },
        _ => panic!("expected Success"),
    }
}

#[test]
fn demo_noop_has_control_kind_with_default_ports() {
    // NoOp uses default output ports (one main output) → a non-terminal
    // control node.
    let adapter = ControlActionAdapter::new(DemoNoOp).expect("valid test catalog definition");
    let meta = adapter.metadata();
    assert_eq!(meta.kind(), ActionKind::Control);
    assert!(!meta.outputs().is_empty(), "NoOp is not a terminal sink");
}

// ── DemoStop ─ explicit success termination ────────────────────────────────

struct DemoStop {
    note: Option<String>,
}

impl DemoStop {
    fn new(note: Option<&str>) -> Self {
        Self {
            note: note.map(ToOwned::to_owned),
        }
    }
}

impl Action for DemoStop {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("demo.stop"),
            nebula_action::metadata_name!("Stop"),
            "Terminate execution with success",
        )
        .with_outputs(Vec::new())
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl ControlAction for DemoStop {
    async fn evaluate(
        &self,
        _input: serde_json::Value,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ControlOutcome<serde_json::Value>, ActionError> {
        Ok(ControlOutcome::Terminate {
            reason: TerminationReason::Success {
                note: self.note.clone(),
            },
        })
    }
}

#[tokio::test]
async fn demo_stop_terminates_with_success() {
    let adapter = ControlActionAdapter::new(DemoStop::new(Some("duplicate detected")))
        .expect("valid test catalog definition");
    let result = run(&adapter, serde_json::json!({})).await;
    match result {
        ActionResult::Terminate { reason } => match reason {
            TerminationReason::Success { note } => {
                assert_eq!(note.as_deref(), Some("duplicate detected"));
            },
            TerminationReason::Failure { .. } => panic!("expected Success"),
            _ => panic!("unexpected TerminationReason variant"),
        },
        _ => panic!("expected Terminate"),
    }
}

#[test]
fn demo_stop_is_terminal_control_node() {
    // Stop keeps `ActionKind::Control`; terminality is carried by its empty
    // `outputs` set, which the validator reads to recognise the graph sink.
    let adapter =
        ControlActionAdapter::new(DemoStop::new(None)).expect("valid test catalog definition");
    let meta = adapter.metadata();
    assert_eq!(meta.kind(), ActionKind::Control);
    assert!(meta.outputs().is_empty(), "Stop is a terminal sink");
}

// ── DemoFail ─ explicit error termination ──────────────────────────────────

struct DemoFail {
    code: String,
    message: String,
}

impl DemoFail {
    fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_owned(),
            message: message.to_owned(),
        }
    }
}

impl Action for DemoFail {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("demo.fail"),
            nebula_action::metadata_name!("Fail"),
            "Terminate execution with failure",
        )
        .with_outputs(Vec::new())
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl ControlAction for DemoFail {
    async fn evaluate(
        &self,
        _input: serde_json::Value,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ControlOutcome<serde_json::Value>, ActionError> {
        Ok(ControlOutcome::Terminate {
            reason: TerminationReason::Failure {
                code: self.code.as_str().into(),
                message: self.message.clone(),
            },
        })
    }
}

#[tokio::test]
async fn demo_fail_terminates_with_failure() {
    let adapter =
        ControlActionAdapter::new(DemoFail::new("E_VALIDATION", "input failed business rules"))
            .expect("valid test catalog definition");
    let result = run(&adapter, serde_json::json!({})).await;
    match result {
        ActionResult::Terminate { reason } => match reason {
            TerminationReason::Failure { code, message } => {
                assert_eq!(code.as_str(), "E_VALIDATION");
                assert_eq!(message, "input failed business rules");
            },
            TerminationReason::Success { .. } => panic!("expected Failure"),
            _ => panic!("unexpected TerminationReason variant"),
        },
        _ => panic!("expected Terminate"),
    }
}

#[test]
fn demo_fail_is_terminal_control_node() {
    let adapter =
        ControlActionAdapter::new(DemoFail::new("E", "m")).expect("valid test catalog definition");
    let meta = adapter.metadata();
    assert_eq!(meta.kind(), ActionKind::Control);
    assert!(meta.outputs().is_empty(), "Fail is a terminal sink");
}

// ── Cross-cutting: all seven fixtures are dyn-compatible and registerable ──

#[test]
fn all_demo_fixtures_are_dyn_control_handles() {
    // Collect all seven demos into a homogeneous Vec<Arc<dyn ControlHandle>>.
    // This simulates what a downstream crate's `register_core_control_nodes`
    // helper would do when populating the ActionRegistry.
    let handlers: Vec<Arc<dyn ControlHandle>> = vec![
        Arc::new(ControlActionAdapter::new(DemoIf).expect("valid test catalog definition")),
        Arc::new(ControlActionAdapter::new(DemoSwitch).expect("valid test catalog definition")),
        Arc::new(
            ControlActionAdapter::new(DemoRouter::new(RouterMode::FirstMatch))
                .expect("valid test catalog definition"),
        ),
        Arc::new(ControlActionAdapter::new(DemoFilter).expect("valid test catalog definition")),
        Arc::new(ControlActionAdapter::new(DemoNoOp).expect("valid test catalog definition")),
        Arc::new(
            ControlActionAdapter::new(DemoStop::new(None)).expect("valid test catalog definition"),
        ),
        Arc::new(
            ControlActionAdapter::new(DemoFail::new("E", "m"))
                .expect("valid test catalog definition"),
        ),
    ];
    assert_eq!(handlers.len(), 7);

    // Each must report its own distinct action key.
    let keys: Vec<_> = handlers
        .iter()
        .map(|h| h.metadata().base().key().clone())
        .collect();
    let unique: std::collections::HashSet<_> = keys.iter().collect();
    assert_eq!(unique.len(), 7, "action keys must be distinct");
}

#[test]
fn kind_and_terminality_inference_matches_expectation() {
    // Every control node is stamped `ActionKind::Control`. Terminality is no
    // longer a distinct kind: it is read structurally from an empty `outputs`
    // set, so the `is_terminal` column maps to `outputs.is_empty()`.
    let cases: &[(Arc<dyn ControlHandle>, bool)] = &[
        (
            Arc::new(ControlActionAdapter::new(DemoIf).expect("valid test catalog definition")),
            false,
        ),
        (
            Arc::new(ControlActionAdapter::new(DemoSwitch).expect("valid test catalog definition")),
            false,
        ),
        (
            Arc::new(ControlActionAdapter::new(DemoFilter).expect("valid test catalog definition")),
            false,
        ),
        (
            Arc::new(ControlActionAdapter::new(DemoNoOp).expect("valid test catalog definition")),
            false,
        ),
        (
            Arc::new(
                ControlActionAdapter::new(DemoStop::new(None))
                    .expect("valid test catalog definition"),
            ),
            true,
        ),
        (
            Arc::new(
                ControlActionAdapter::new(DemoFail::new("E", "m"))
                    .expect("valid test catalog definition"),
            ),
            true,
        ),
    ];
    for (handler, is_terminal) in cases {
        let meta = handler.metadata();
        assert_eq!(
            meta.kind(),
            ActionKind::Control,
            "every control node is `Control`: {}",
            meta.base().key().clone()
        );
        assert_eq!(
            meta.outputs().is_empty(),
            *is_terminal,
            "terminality (empty outputs) mismatch for {}",
            meta.base().key().clone()
        );
    }
}

// ── Generic accept-any-ControlAction function test ─────────────────────────

/// Generic helper proving that `impl ControlAction` is usable as a
/// compile-time trait bound. This is the main reason the trait exists as
/// a public contract rather than a macro — community code can write
/// functions parameterized by `impl ControlAction`.
fn wrap_and_execute<A: ControlAction>(action: A) -> Arc<dyn ControlHandle> {
    Arc::new(ControlActionAdapter::new(action).expect("valid test catalog definition"))
}

#[tokio::test]
async fn generic_bound_accepts_any_control_action() {
    let h = wrap_and_execute(DemoIf);
    let ctx = make_ctx();
    let input = h
        .prepare_input(ActionInput::Raw(serde_json::json!({ "condition": true })))
        .unwrap();
    let result = h.dispatch(input, &ctx).await.unwrap();
    assert!(matches!(result, ActionResult::Branch { .. }));
}

// ── Drop vs Success: output shape differences ──────────────────────────────

#[tokio::test]
async fn pass_and_drop_have_distinct_runtime_shapes() {
    // Pass carries an output in ActionOutput, Drop has no output at all.
    // Downstream consumers (engine, journal) must distinguish these shapes.
    let filter = ControlActionAdapter::new(DemoFilter).expect("valid test catalog definition");

    let pass_result = run(&filter, serde_json::json!({ "score": 100 })).await;
    match pass_result {
        ActionResult::Success { output } => {
            assert!(matches!(output, ActionOutput::Value(_)));
        },
        _ => panic!("expected Success"),
    }

    let drop_result = run(&filter, serde_json::json!({ "score": 0 })).await;
    assert!(drop_result.into_primary_output().is_none());
}

// ── output_schema stamp: ControlActionAdapter ──────────────────────────────
//
// Every control action wrapped by `ControlActionAdapter` must carry a
// non-empty `output_schema` when its `Output` type has typed fields.
// T3 (TypeDAG edge check) reads `metadata().output_schema()` to validate
// producer→consumer assignability; an empty schema here silently bypasses
// the check for every control node.
//
// The fixture below uses a typed `Output` struct so the assertion is
// non-vacuous: removing the `output_schema` stamp from
// `ControlActionAdapter::new` causes this test to go RED.

/// Typed output for the control-adapter stamp test.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct TypedBranchOutput {
    /// The selected branch label, carried downstream for audit / tracing.
    selected: String,
}

impl HasSchema for TypedBranchOutput {
    fn schema() -> Result<ValidSchema, nebula_schema::ValidationReport> {
        Schema::builder()
            .string(field_key!("selected"), StringBuilder::required)
            .build()
    }
}

/// Control action that returns a typed `Output` so the factory-stamped
/// `output_schema` is non-empty and the red-on-revert assertion is non-vacuous.
struct DemoTypedBranch;

impl Action for DemoTypedBranch {
    type Input = serde_json::Value;
    type Output = TypedBranchOutput;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("demo.typed_branch"),
            nebula_action::metadata_name!("TypedBranch"),
            "Branch with typed output",
        )
        .with_outputs(vec![
            OutputPort::flow(port_key!("true")),
            OutputPort::flow(port_key!("false")),
        ])
    }

    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}

impl ControlAction for DemoTypedBranch {
    async fn evaluate(
        &self,
        input: serde_json::Value,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ControlOutcome<TypedBranchOutput>, ActionError> {
        let branch_name = if required_bool(&input, "/condition")? {
            "true"
        } else {
            "false"
        };
        let selected =
            BranchKey::new(branch_name).expect("'true'/'false' are valid branch key literals");
        Ok(ControlOutcome::Branch {
            selected,
            output: TypedBranchOutput {
                selected: branch_name.into(),
            },
        })
    }
}

#[test]
fn control_action_adapter_stamps_output_schema_from_action_output_type() {
    // Non-vacuous: `TypedBranchOutput` has a `selected` field.
    // Removing the `output_schema` stamp from `ControlActionAdapter::new`
    // causes this test to go RED — the schema will be empty and the `any()`
    // predicate will return false.
    let adapter =
        ControlActionAdapter::new(DemoTypedBranch).expect("valid test catalog definition");
    let output_schema = adapter.metadata().output_schema();

    assert!(
        output_schema
            .fields()
            .iter()
            .any(|f| f.key().as_str() == "selected"),
        "ControlActionAdapter must stamp output_schema from A::Output — \
         `selected` field missing; revert the stamp in ControlActionAdapter::new to see this fail"
    );
}
