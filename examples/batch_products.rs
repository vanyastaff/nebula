//! `BatchAction` DX trait demo — batch-fetches dummyjson products by ID.
//!
//! Run:
//!
//! ```bash
//! cargo run -p nebula-examples --bin batch_products
//! ```
//!
//! Most IDs are valid; one (`9999`) returns HTTP 404 and surfaces as
//! `BatchItemResult::Failed` without aborting the rest of the batch.
//! The default `batch_size = 5` chunks the 13 items into 3 iterations.

use nebula_sdk::prelude::{
    Action, ActionContext, ActionError, ActionMetadata, BatchAction, BatchItemResult,
    DeclaresDependencies, Deserialize, TestContextBuilder, TestRuntime, Value, action_key,
    impl_batch_action, json,
};

// ── Action definition ──────────────────────────────────────────────────────

struct DummyJsonProductsBatchAction {
    meta: ActionMetadata,
}

impl DummyJsonProductsBatchAction {
    fn new() -> Self {
        Self {
            meta: ActionMetadata::new(
                action_key!("dummyjson.products_batch"),
                "DummyJSON Products Batch",
                "Batch-fetch products by ID from dummyjson via BatchAction DX trait",
            ),
        }
    }
}

impl DeclaresDependencies for DummyJsonProductsBatchAction {}
impl Action for DummyJsonProductsBatchAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

#[derive(Deserialize)]
struct DummyJsonProductRaw {
    id: u32,
    title: String,
    price: f64,
    category: String,
    rating: f64,
}

impl BatchAction for DummyJsonProductsBatchAction {
    type Input = Value;
    type Item = u32;
    type Output = Value;

    fn batch_size(&self) -> usize {
        5
    }

    fn extract_items(&self, input: &Self::Input) -> Vec<u32> {
        input.get("ids").and_then(|v| v.as_array()).map_or_else(
            || vec![1, 2, 3, 4, 5, 6, 7, 8, 9999, 9, 10, 11, 12],
            |arr| {
                arr.iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u32))
                    .collect()
            },
        )
    }

    async fn process_item(
        &self,
        item: u32,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<Value, ActionError> {
        let url = format!("https://dummyjson.com/products/{item}");
        let response = reqwest::get(&url)
            .await
            .map_err(|e| ActionError::retryable(format!("HTTP error for product {item}: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            // Must be non-fatal so the macro captures it as a per-item failure.
            return Err(ActionError::retryable(format!(
                "HTTP {status} for product {item}"
            )));
        }

        let raw: DummyJsonProductRaw = response
            .json()
            .await
            .map_err(|e| ActionError::retryable(format!("parse error for product {item}: {e}")))?;

        Ok(json!({
            "id": raw.id,
            "title": raw.title,
            "price": raw.price,
            "category": raw.category,
            "rating": raw.rating,
        }))
    }

    fn merge_results(&self, results: Vec<BatchItemResult<Value>>) -> Value {
        let mut products = Vec::new();
        let mut errors = Vec::new();
        for r in results {
            match r {
                BatchItemResult::Ok { output } => products.push(output),
                BatchItemResult::Failed { error } => errors.push(error),
            }
        }
        json!({
            "ok_count": products.len(),
            "failed_count": errors.len(),
            "products": products,
            "errors": errors,
        })
    }
}

impl_batch_action!(DummyJsonProductsBatchAction);

// ── Runner ─────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("▶ batch_products — fetching 13 dummyjson products in chunks of 5");
    println!();

    let ctx = TestContextBuilder::new().with_input(json!({}));
    let report = TestRuntime::new(ctx)
        .run_stateful(DummyJsonProductsBatchAction::new())
        .await?;

    println!("kind:       {}", report.kind);
    println!("iterations: {}", report.iterations);
    println!("duration:   {:?}", report.duration);
    if let Some(note) = &report.note {
        println!("note:       {note}");
    }
    println!();
    println!("Final merged result:");
    println!("{}", serde_json::to_string_pretty(&report.output)?);

    Ok(())
}
