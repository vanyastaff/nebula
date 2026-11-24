//! DX tests for `BatchAction` trait and `impl_batch_action!` macro.
//!
//! Validates that the macro-generated `StatefulAction` impl correctly drives
//! fixed-size batch processing through the `StatefulTestHarness`.

use nebula_action::{
    ActionContext,
    action::Action,
    error::ActionError,
    result::ActionResult,
    stateful::{BatchAction, BatchItemResult},
    testing::{StatefulTestHarness, TestContextBuilder},
};
use nebula_schema::{HasSchema, ValidSchema};
use serde::{Deserialize, Serialize};

// ── DoublerBatch ──────────────────────────────────────────────────────────

struct DoublerBatch;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NumberList {
    numbers: Vec<i32>,
}

impl HasSchema for NumberList {
    fn schema() -> Result<ValidSchema, nebula_schema::ValidationReport> {
        use nebula_schema::{PropertyCollector, Schema, field_key};
        Schema::builder()
            .list(field_key!("numbers"), |l| {
                l.item_number(field_key!("n"), nebula_schema::NumberBuilder::integer)
            })
            .build()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, nebula_schema::Schema)]
struct BatchOutput {
    processed: Vec<i32>,
    errors: u64,
}

impl nebula_action::Component for DoublerBatch {
    type Kind = nebula_action::kind::Action;
    type Config = NumberList;

    const KEY: &'static str = "test.doubler_batch";

    fn metadata() -> nebula_action::Metadata {
        nebula_action::Metadata::new(
            nebula_action::metadata_name!("DoublerBatch"),
            "Batch doubler",
        )
    }
}

impl Action for DoublerBatch {
    type Output = BatchOutput;
}

impl BatchAction for DoublerBatch {
    type Item = i32;

    fn batch_size(&self) -> usize {
        3
    }

    fn extract_items(&self, input: &NumberList) -> Vec<i32> {
        input.numbers.clone()
    }

    async fn process_item(
        &self,
        item: i32,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<BatchOutput, ActionError> {
        if item < 0 {
            return Err(ActionError::retryable(format!("negative: {item}")));
        }
        Ok(BatchOutput {
            processed: vec![item * 2],
            errors: 0,
        })
    }

    fn merge_results(&self, results: Vec<BatchItemResult<BatchOutput>>) -> BatchOutput {
        let mut processed = Vec::new();
        let mut errors = 0;
        for r in results {
            match r {
                BatchItemResult::Ok { output } => processed.extend(output.processed),
                BatchItemResult::Failed { .. } => errors += 1,
                // `BatchItemResult` is `#[non_exhaustive]`; future variants are treated as failures.
                _ => errors += 1,
            }
        }
        BatchOutput { processed, errors }
    }
}

nebula_action::impl_batch_action!(DoublerBatch);

// ── Tests ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn batch_processes_in_chunks() {
    let action = DoublerBatch;
    let ctx = TestContextBuilder::minimal().build();
    let mut harness = StatefulTestHarness::new(action, ctx).unwrap();

    let input = NumberList {
        numbers: (1..=10).collect(),
    };

    // 10 items, batch_size=3 → chunks of [3, 3, 3, 1] = 4 steps
    let r1 = harness.step(input.clone()).await.unwrap();
    assert!(r1.is_continue(), "chunk 1 of 4 should Continue");

    let r2 = harness.step(input.clone()).await.unwrap();
    assert!(r2.is_continue(), "chunk 2 of 4 should Continue");

    let r3 = harness.step(input.clone()).await.unwrap();
    assert!(r3.is_continue(), "chunk 3 of 4 should Continue");

    let r4 = harness.step(input).await.unwrap();
    assert!(
        matches!(r4, ActionResult::Break { .. }),
        "chunk 4 of 4 should Break"
    );

    assert_eq!(harness.iterations(), 4);
}

#[tokio::test]
async fn batch_handles_per_item_errors() {
    let action = DoublerBatch;
    let ctx = TestContextBuilder::minimal().build();
    let mut harness = StatefulTestHarness::new(action, ctx).unwrap();

    // 3 items fit in one chunk (batch_size=3), -1 will fail
    let input = NumberList {
        numbers: vec![1, -1, 2],
    };

    let r = harness.step(input).await.unwrap();
    match r {
        ActionResult::Break { output, .. } => {
            let result: BatchOutput = output.into_value().unwrap();
            assert_eq!(result.processed, vec![2, 4]);
            assert_eq!(result.errors, 1);
        },
        other => panic!("expected Break, got {other:?}"),
    }
}

#[tokio::test]
async fn batch_single_chunk() {
    let action = DoublerBatch;
    let ctx = TestContextBuilder::minimal().build();
    let mut harness = StatefulTestHarness::new(action, ctx).unwrap();

    let input = NumberList { numbers: vec![5] };

    let r = harness.step(input).await.unwrap();
    assert!(
        matches!(r, ActionResult::Break { .. }),
        "single item should Break immediately"
    );

    assert_eq!(harness.iterations(), 1);
}
