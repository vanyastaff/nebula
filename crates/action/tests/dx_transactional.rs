//! DX tests for `TransactionalAction` trait and `impl_transactional_action!` macro.
//!
//! Validates that the macro-generated `StatefulAction` impl correctly drives
//! saga-pattern transactional execution through the `StatefulTestHarness`.

use nebula_action::action::Action;
use nebula_action::context::Context;
use nebula_action::dependency::ActionDependencies;
use nebula_action::error::ActionError;
use nebula_action::metadata::ActionMetadata;
use nebula_action::result::{ActionResult, BreakReason};
use nebula_action::stateful::TransactionalAction;
use nebula_action::testing::{StatefulTestHarness, TestContextBuilder};
use nebula_core::action_key;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

// ── PaymentAction ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Confirmation {
    tx_id: String,
    amount: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RefundInfo {
    tx_id: String,
}

struct PaymentAction {
    meta: ActionMetadata,
    compensated: Arc<AtomicBool>,
}

impl ActionDependencies for PaymentAction {}

impl Action for PaymentAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl TransactionalAction for PaymentAction {
    type Input = serde_json::Value;
    type Output = Confirmation;
    type CompensationData = RefundInfo;

    async fn execute_tx(
        &self,
        _input: serde_json::Value,
        _ctx: &impl Context,
    ) -> Result<(Confirmation, RefundInfo), ActionError> {
        Ok((
            Confirmation {
                tx_id: "tx_123".into(),
                amount: 1000,
            },
            RefundInfo {
                tx_id: "tx_123".into(),
            },
        ))
    }

    async fn compensate(&self, _data: RefundInfo, _ctx: &impl Context) -> Result<(), ActionError> {
        self.compensated.store(true, Ordering::Relaxed);
        Ok(())
    }
}

nebula_action::impl_transactional_action!(PaymentAction);

// ── Helpers ──────────────────────────────────────────────────────────────

fn make_payment_action() -> (PaymentAction, Arc<AtomicBool>) {
    let compensated = Arc::new(AtomicBool::new(false));
    let action = PaymentAction {
        meta: ActionMetadata::new(action_key!("test.payment"), "PaymentAction", "Test payment"),
        compensated: Arc::clone(&compensated),
    };
    (action, compensated)
}

// ── Tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn transactional_forward_execution() {
    let (action, compensated) = make_payment_action();
    let ctx = TestContextBuilder::minimal().build();
    let mut harness = StatefulTestHarness::new(action, ctx).unwrap();

    let r = harness.step(serde_json::json!({})).await.unwrap();

    match r {
        ActionResult::Break { output, reason } => {
            assert_eq!(reason, BreakReason::Completed);
            let val: Confirmation = output.into_value().unwrap();
            assert_eq!(val.tx_id, "tx_123");
            assert_eq!(val.amount, 1000);
        }
        other => panic!("expected Break(Completed), got {other:?}"),
    }

    assert!(!compensated.load(Ordering::Relaxed));
}

#[tokio::test]
async fn transactional_compensation() {
    let (action, compensated) = make_payment_action();
    let ctx = TestContextBuilder::minimal().build();
    let mut harness = StatefulTestHarness::new(action, ctx).unwrap();

    // Step 1: forward execution
    let r1 = harness.step(serde_json::json!({})).await.unwrap();
    assert!(
        matches!(r1, ActionResult::Break { ref reason, .. } if *reason == BreakReason::Completed),
        "step 1 should Break(Completed)"
    );

    // Step 2: triggers compensation
    let r2 = harness.step(serde_json::json!({})).await.unwrap();
    match r2 {
        ActionResult::Break { reason, .. } => {
            assert_eq!(reason, BreakReason::Custom("compensated".into()));
        }
        other => panic!("expected Break(Custom(\"compensated\")), got {other:?}"),
    }

    assert!(compensated.load(Ordering::Relaxed));
}

#[tokio::test]
async fn transactional_double_compensation_fails() {
    let (action, _compensated) = make_payment_action();
    let ctx = TestContextBuilder::minimal().build();
    let mut harness = StatefulTestHarness::new(action, ctx).unwrap();

    // Step 1: forward execution
    let _r1 = harness.step(serde_json::json!({})).await.unwrap();

    // Step 2: compensation
    let _r2 = harness.step(serde_json::json!({})).await.unwrap();

    // Step 3: already compensated → should return Err(Fatal)
    let r3 = harness.step(serde_json::json!({})).await;
    nebula_action::assert_fatal!(r3);
}
