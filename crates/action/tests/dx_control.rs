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

use std::sync::Arc;

use nebula_action::{
    Action, ActionCategory, ActionError, ActionMetadata, ActionOutput, ActionResult, ControlAction,
    ControlActionAdapter, ControlInput, ControlOutcome, OutputPort, StatelessHandler,
    TerminationReason, ValidationReason,
    testing::{TestActionContext as ActionContext, TestContextBuilder},
};
use nebula_core::{DeclaresDependencies, action_key};

// ── Test helpers ───────────────────────────────────────────────────────────

fn make_ctx() -> ActionContext {
    TestContextBuilder::new().build()
}

async fn run(
    adapter: &impl StatelessHandler,
    input: serde_json::Value,
) -> ActionResult<serde_json::Value> {
    let ctx = make_ctx();
    StatelessHandler::execute(adapter, input, &ctx)
        .await
        .expect("execute should succeed")
}

async fn run_err(adapter: &impl StatelessHandler, input: serde_json::Value) -> ActionError {
    let ctx = make_ctx();
    StatelessHandler::execute(adapter, input, &ctx)
        .await
        .expect_err("execute should fail")
}

// ── DemoIf ─ binary branch ─────────────────────────────────────────────────

struct DemoIf {
    metadata: ActionMetadata,
}

impl DemoIf {
    fn new() -> Self {
        Self {
            metadata: ActionMetadata::new(action_key!("demo.if"), "If", "Binary branch")
                .with_outputs(vec![OutputPort::flow("true"), OutputPort::flow("false")]),
        }
    }
}

impl DeclaresDependencies for DemoIf {}
impl Action for DemoIf {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl ControlAction for DemoIf {
    async fn evaluate(
        &self,
        input: ControlInput,
        _ctx: &nebula_action::ActionContext,
    ) -> Result<ControlOutcome, ActionError> {
        let condition = input.get_bool("/condition")?;
        let selected = if condition { "true" } else { "false" };
        Ok(ControlOutcome::Branch {
            selected: selected.into(),
            output: input.into_value(),
        })
    }
}

#[tokio::test]
async fn demo_if_routes_true() {
    let adapter = ControlActionAdapter::new(DemoIf::new());
    let result = run(
        &adapter,
        serde_json::json!({ "condition": true, "value": 42 }),
    )
    .await;
    match result {
        ActionResult::Branch {
            selected, output, ..
        } => {
            assert_eq!(selected, "true");
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
    let adapter = ControlActionAdapter::new(DemoIf::new());
    let result = run(&adapter, serde_json::json!({ "condition": false })).await;
    match result {
        ActionResult::Branch { selected, .. } => assert_eq!(selected, "false"),
        _ => panic!("expected Branch"),
    }
}

#[tokio::test]
async fn demo_if_missing_condition_is_validation_error() {
    let adapter = ControlActionAdapter::new(DemoIf::new());
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
    let adapter = ControlActionAdapter::new(DemoIf::new());
    let err = run_err(&adapter, serde_json::json!({ "condition": "yes" })).await;
    match err {
        ActionError::Validation { reason, .. } => {
            assert_eq!(reason, ValidationReason::WrongType);
        },
        _ => panic!("expected Validation error"),
    }
}

#[test]
fn demo_if_has_control_category() {
    let adapter = ControlActionAdapter::new(DemoIf::new());
    assert_eq!(adapter.metadata().category, ActionCategory::Control);
}

// ── DemoSwitch ─ N-way static branch ───────────────────────────────────────

struct DemoSwitch {
    metadata: ActionMetadata,
}

impl DemoSwitch {
    fn new() -> Self {
        Self {
            metadata: ActionMetadata::new(
                action_key!("demo.switch"),
                "Switch",
                "N-way branch by status field",
            )
            .with_outputs(vec![
                OutputPort::flow("active"),
                OutputPort::flow("pending"),
                OutputPort::flow("archived"),
                OutputPort::flow("default"),
            ]),
        }
    }
}

impl DeclaresDependencies for DemoSwitch {}
impl Action for DemoSwitch {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl ControlAction for DemoSwitch {
    async fn evaluate(
        &self,
        input: ControlInput,
        _ctx: &nebula_action::ActionContext,
    ) -> Result<ControlOutcome, ActionError> {
        let status = input.get_str("/status")?;
        let selected = match status {
            "active" | "pending" | "archived" => status,
            _ => "default",
        };
        Ok(ControlOutcome::Branch {
            selected: selected.into(),
            output: input.into_value(),
        })
    }
}

#[tokio::test]
async fn demo_switch_routes_known_case() {
    let adapter = ControlActionAdapter::new(DemoSwitch::new());
    let result = run(&adapter, serde_json::json!({ "status": "pending" })).await;
    match result {
        ActionResult::Branch { selected, .. } => assert_eq!(selected, "pending"),
        _ => panic!("expected Branch"),
    }
}

#[tokio::test]
async fn demo_switch_falls_back_to_default() {
    let adapter = ControlActionAdapter::new(DemoSwitch::new());
    let result = run(&adapter, serde_json::json!({ "status": "unknown" })).await;
    match result {
        ActionResult::Branch { selected, .. } => assert_eq!(selected, "default"),
        _ => panic!("expected Branch"),
    }
}

// ── DemoRouter ─ multi-match routing ───────────────────────────────────────

struct DemoRouter {
    metadata: ActionMetadata,
    mode: RouterMode,
}

#[derive(Clone, Copy)]
enum RouterMode {
    FirstMatch,
    AllMatch,
}

impl DemoRouter {
    fn new(mode: RouterMode) -> Self {
        Self {
            metadata: ActionMetadata::new(
                action_key!("demo.router"),
                "Router",
                "Multi-rule routing",
            )
            .with_outputs(vec![
                OutputPort::flow("high"),
                OutputPort::flow("medium"),
                OutputPort::flow("low"),
            ]),
            mode,
        }
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

impl DeclaresDependencies for DemoRouter {}
impl Action for DemoRouter {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl ControlAction for DemoRouter {
    async fn evaluate(
        &self,
        input: ControlInput,
        _ctx: &nebula_action::ActionContext,
    ) -> Result<ControlOutcome, ActionError> {
        let priority = input.get_i64("/priority")?;
        let matches = Self::classify(priority);
        match (self.mode, matches.as_slice()) {
            (_, []) => Ok(ControlOutcome::Drop {
                reason: Some(format!("no rule matched priority {priority}")),
            }),
            (RouterMode::FirstMatch, _) => {
                let value = input.into_value();
                Ok(ControlOutcome::Branch {
                    selected: matches[0].into(),
                    output: value,
                })
            },
            (RouterMode::AllMatch, _) => {
                let value = input.into_value();
                let ports: std::collections::HashMap<_, _> = matches
                    .iter()
                    .map(|port| ((*port).to_string(), value.clone()))
                    .collect();
                Ok(ControlOutcome::Route { ports })
            },
        }
    }
}

#[tokio::test]
async fn demo_router_first_match_picks_first_rule() {
    let adapter = ControlActionAdapter::new(DemoRouter::new(RouterMode::FirstMatch));
    let result = run(&adapter, serde_json::json!({ "priority": 150 })).await;
    match result {
        ActionResult::Branch { selected, .. } => assert_eq!(selected, "high"),
        _ => panic!("expected Branch"),
    }
}

#[tokio::test]
async fn demo_router_all_match_fires_multiple_ports() {
    let adapter = ControlActionAdapter::new(DemoRouter::new(RouterMode::AllMatch));
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
struct NeverMatchRouter {
    metadata: ActionMetadata,
}

impl NeverMatchRouter {
    fn new() -> Self {
        Self {
            metadata: ActionMetadata::new(
                action_key!("demo.never_match_router"),
                "NeverMatchRouter",
                "Drops every input — used to test Drop code path",
            )
            .with_outputs(vec![OutputPort::flow("out")]),
        }
    }
}

impl DeclaresDependencies for NeverMatchRouter {}
impl Action for NeverMatchRouter {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl ControlAction for NeverMatchRouter {
    async fn evaluate(
        &self,
        _input: ControlInput,
        _ctx: &nebula_action::ActionContext,
    ) -> Result<ControlOutcome, ActionError> {
        Ok(ControlOutcome::Drop {
            reason: Some("sentinel".into()),
        })
    }
}

#[tokio::test]
async fn demo_router_no_rules_match_drops() {
    let adapter = ControlActionAdapter::new(NeverMatchRouter::new());
    let result = run(&adapter, serde_json::json!({ "priority": 999 })).await;
    match result {
        ActionResult::Drop { reason } => assert_eq!(reason.as_deref(), Some("sentinel")),
        _ => panic!("expected Drop"),
    }
}

// ── DemoFilter ─ drop-or-pass gate ─────────────────────────────────────────

struct DemoFilter {
    metadata: ActionMetadata,
}

impl DemoFilter {
    fn new() -> Self {
        Self {
            metadata: ActionMetadata::new(
                action_key!("demo.filter"),
                "Filter",
                "Drop items below threshold",
            )
            .with_outputs(vec![OutputPort::flow("out")]),
        }
    }
}

impl DeclaresDependencies for DemoFilter {}
impl Action for DemoFilter {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl ControlAction for DemoFilter {
    async fn evaluate(
        &self,
        input: ControlInput,
        _ctx: &nebula_action::ActionContext,
    ) -> Result<ControlOutcome, ActionError> {
        let score = input.get_i64("/score")?;
        if score >= 50 {
            Ok(ControlOutcome::Pass {
                output: input.into_value(),
            })
        } else {
            Ok(ControlOutcome::Drop {
                reason: Some(format!("score {score} below 50")),
            })
        }
    }
}

#[tokio::test]
async fn demo_filter_passes_above_threshold() {
    let adapter = ControlActionAdapter::new(DemoFilter::new());
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
    let adapter = ControlActionAdapter::new(DemoFilter::new());
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
    let adapter = ControlActionAdapter::new(DemoFilter::new());
    let result = run(&adapter, serde_json::json!({ "score": 5 })).await;
    assert!(result.is_drop(), "Filter must use Drop, not Skip");
    assert!(!matches!(result, ActionResult::Skip { .. }));
}

// ── DemoNoOp ─ pure passthrough ────────────────────────────────────────────

struct DemoNoOp {
    metadata: ActionMetadata,
}

impl DemoNoOp {
    fn new() -> Self {
        Self {
            metadata: ActionMetadata::new(
                action_key!("demo.noop"),
                "NoOp",
                "Pass-through placeholder",
            ),
        }
    }
}

impl DeclaresDependencies for DemoNoOp {}
impl Action for DemoNoOp {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl ControlAction for DemoNoOp {
    async fn evaluate(
        &self,
        input: ControlInput,
        _ctx: &nebula_action::ActionContext,
    ) -> Result<ControlOutcome, ActionError> {
        Ok(ControlOutcome::Pass {
            output: input.into_value(),
        })
    }
}

#[tokio::test]
async fn demo_noop_preserves_input() {
    let adapter = ControlActionAdapter::new(DemoNoOp::new());
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
fn demo_noop_has_control_category_with_default_port() {
    // NoOp uses default output ports (one main output) → Control, not Terminal.
    let adapter = ControlActionAdapter::new(DemoNoOp::new());
    assert_eq!(adapter.metadata().category, ActionCategory::Control);
}

// ── DemoStop ─ explicit success termination ────────────────────────────────

struct DemoStop {
    metadata: ActionMetadata,
    note: Option<String>,
}

impl DemoStop {
    fn new(note: Option<&str>) -> Self {
        Self {
            metadata: ActionMetadata::new(
                action_key!("demo.stop"),
                "Stop",
                "Terminate execution with success",
            )
            .with_outputs(Vec::new()),
            note: note.map(ToOwned::to_owned),
        }
    }
}

impl DeclaresDependencies for DemoStop {}
impl Action for DemoStop {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl ControlAction for DemoStop {
    async fn evaluate(
        &self,
        _input: ControlInput,
        _ctx: &nebula_action::ActionContext,
    ) -> Result<ControlOutcome, ActionError> {
        Ok(ControlOutcome::Terminate {
            reason: TerminationReason::Success {
                note: self.note.clone(),
            },
        })
    }
}

#[tokio::test]
async fn demo_stop_terminates_with_success() {
    let adapter = ControlActionAdapter::new(DemoStop::new(Some("duplicate detected")));
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
fn demo_stop_has_terminal_category() {
    // Zero output ports → Terminal subcategory.
    let adapter = ControlActionAdapter::new(DemoStop::new(None));
    assert_eq!(adapter.metadata().category, ActionCategory::Terminal);
}

// ── DemoFail ─ explicit error termination ──────────────────────────────────

struct DemoFail {
    metadata: ActionMetadata,
    code: String,
    message: String,
}

impl DemoFail {
    fn new(code: &str, message: &str) -> Self {
        Self {
            metadata: ActionMetadata::new(
                action_key!("demo.fail"),
                "Fail",
                "Terminate execution with failure",
            )
            .with_outputs(Vec::new()),
            code: code.to_owned(),
            message: message.to_owned(),
        }
    }
}

impl DeclaresDependencies for DemoFail {}
impl Action for DemoFail {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl ControlAction for DemoFail {
    async fn evaluate(
        &self,
        _input: ControlInput,
        _ctx: &nebula_action::ActionContext,
    ) -> Result<ControlOutcome, ActionError> {
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
        ControlActionAdapter::new(DemoFail::new("E_VALIDATION", "input failed business rules"));
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
fn demo_fail_has_terminal_category() {
    let adapter = ControlActionAdapter::new(DemoFail::new("E", "m"));
    assert_eq!(adapter.metadata().category, ActionCategory::Terminal);
}

// ── Cross-cutting: all seven fixtures are dyn-compatible and registerable ──

#[test]
fn all_demo_fixtures_are_dyn_stateless_handlers() {
    // Collect all seven demos into a homogeneous Vec<Arc<dyn StatelessHandler>>.
    // This simulates what a downstream crate's `register_core_control_nodes`
    // helper would do when populating the ActionRegistry.
    let handlers: Vec<Arc<dyn StatelessHandler>> = vec![
        Arc::new(ControlActionAdapter::new(DemoIf::new())),
        Arc::new(ControlActionAdapter::new(DemoSwitch::new())),
        Arc::new(ControlActionAdapter::new(DemoRouter::new(
            RouterMode::FirstMatch,
        ))),
        Arc::new(ControlActionAdapter::new(DemoFilter::new())),
        Arc::new(ControlActionAdapter::new(DemoNoOp::new())),
        Arc::new(ControlActionAdapter::new(DemoStop::new(None))),
        Arc::new(ControlActionAdapter::new(DemoFail::new("E", "m"))),
    ];
    assert_eq!(handlers.len(), 7);

    // Each must report its own distinct action key.
    let keys: Vec<_> = handlers
        .iter()
        .map(|h| h.metadata().base.key.clone())
        .collect();
    let unique: std::collections::HashSet<_> = keys.iter().collect();
    assert_eq!(unique.len(), 7, "action keys must be distinct");
}

#[test]
fn category_inference_matches_expectation() {
    let cases: &[(Arc<dyn StatelessHandler>, ActionCategory)] = &[
        (
            Arc::new(ControlActionAdapter::new(DemoIf::new())),
            ActionCategory::Control,
        ),
        (
            Arc::new(ControlActionAdapter::new(DemoSwitch::new())),
            ActionCategory::Control,
        ),
        (
            Arc::new(ControlActionAdapter::new(DemoFilter::new())),
            ActionCategory::Control,
        ),
        (
            Arc::new(ControlActionAdapter::new(DemoNoOp::new())),
            ActionCategory::Control,
        ),
        (
            Arc::new(ControlActionAdapter::new(DemoStop::new(None))),
            ActionCategory::Terminal,
        ),
        (
            Arc::new(ControlActionAdapter::new(DemoFail::new("E", "m"))),
            ActionCategory::Terminal,
        ),
    ];
    for (handler, expected) in cases {
        assert_eq!(
            handler.metadata().category,
            *expected,
            "unexpected category for {}",
            handler.metadata().base.key
        );
    }
}

// ── Generic accept-any-ControlAction function test ─────────────────────────

/// Generic helper proving that `impl ControlAction` is usable as a
/// compile-time trait bound. This is the main reason the trait exists as
/// a public contract rather than a macro — community code can write
/// functions parameterized by `impl ControlAction`.
fn wrap_and_execute<A: ControlAction>(action: A) -> Arc<dyn StatelessHandler> {
    Arc::new(ControlActionAdapter::new(action))
}

#[tokio::test]
async fn generic_bound_accepts_any_control_action() {
    let h = wrap_and_execute(DemoIf::new());
    let ctx = make_ctx();
    let result = h
        .execute(serde_json::json!({ "condition": true }), &ctx)
        .await
        .unwrap();
    assert!(matches!(result, ActionResult::Branch { .. }));
}

// ── Drop vs Success: output shape differences ──────────────────────────────

#[tokio::test]
async fn pass_and_drop_have_distinct_runtime_shapes() {
    // Pass carries an output in ActionOutput, Drop has no output at all.
    // Downstream consumers (engine, journal) must distinguish these shapes.
    let filter = ControlActionAdapter::new(DemoFilter::new());

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
