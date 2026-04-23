//! DX tests for `PaginatedAction` trait and `impl_paginated_action!` macro.
//!
//! Validates that the macro-generated `StatefulAction` impl correctly drives
//! cursor-based pagination through the `StatefulTestHarness`.

use nebula_action::{
    action::Action,
    context::ActionContext,
    error::ActionError,
    metadata::ActionMetadata,
    result::ActionResult,
    stateful::{PageResult, PaginatedAction},
    testing::{StatefulTestHarness, TestContextBuilder},
};
use nebula_core::{DeclaresDependencies, action_key};

// ── NumberPaginator ────────────────────────────────────────────────────────

struct NumberPaginator {
    meta: ActionMetadata,
    total_pages: u32,
}

impl DeclaresDependencies for NumberPaginator {}

impl Action for NumberPaginator {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl PaginatedAction for NumberPaginator {
    type Input = serde_json::Value;
    type Output = Vec<i32>;
    type Cursor = u32;

    fn max_pages(&self) -> u32 {
        self.total_pages + 1
    }

    async fn fetch_page(
        &self,
        _input: &serde_json::Value,
        cursor: Option<&u32>,
        _ctx: &ActionContext,
    ) -> Result<PageResult<Vec<i32>, u32>, ActionError> {
        let page = cursor.copied().unwrap_or(0);
        let data: Vec<i32> = ((page * 10)..((page + 1) * 10)).map(|i| i as i32).collect();
        let next = if page + 1 < self.total_pages {
            Some(page + 1)
        } else {
            None
        };
        Ok(PageResult {
            data,
            next_cursor: next,
        })
    }
}

nebula_action::impl_paginated_action!(NumberPaginator);

// ── LimitedPaginator ───────────────────────────────────────────────────────

struct LimitedPaginator {
    meta: ActionMetadata,
    inner: NumberPaginator,
}

impl DeclaresDependencies for LimitedPaginator {}

impl Action for LimitedPaginator {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl PaginatedAction for LimitedPaginator {
    type Input = serde_json::Value;
    type Output = Vec<i32>;
    type Cursor = u32;

    fn max_pages(&self) -> u32 {
        2
    }

    async fn fetch_page(
        &self,
        input: &serde_json::Value,
        cursor: Option<&u32>,
        ctx: &ActionContext,
    ) -> Result<PageResult<Vec<i32>, u32>, ActionError> {
        self.inner.fetch_page(input, cursor, ctx).await
    }
}

nebula_action::impl_paginated_action!(LimitedPaginator);

// ── Tests ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn paginated_fetches_all_pages() {
    let action = NumberPaginator {
        meta: ActionMetadata::new(
            action_key!("test.number_paginator"),
            "NumberPaginator",
            "Paginate numbers",
        ),
        total_pages: 3,
    };
    let ctx = TestContextBuilder::minimal().build();
    let mut harness = StatefulTestHarness::new(action, ctx).unwrap();
    let input = serde_json::json!({});

    let r1 = harness.step(input.clone()).await.unwrap();
    assert!(r1.is_continue(), "page 1 of 3 should Continue");

    let r2 = harness.step(input.clone()).await.unwrap();
    assert!(r2.is_continue(), "page 2 of 3 should Continue");

    let r3 = harness.step(input).await.unwrap();
    assert!(
        matches!(r3, ActionResult::Break { .. }),
        "page 3 of 3 should Break"
    );

    assert_eq!(harness.iterations(), 3);
}

#[tokio::test]
async fn paginated_respects_max_pages() {
    let action = LimitedPaginator {
        meta: ActionMetadata::new(
            action_key!("test.limited_paginator"),
            "LimitedPaginator",
            "Paginate with limit",
        ),
        inner: NumberPaginator {
            meta: ActionMetadata::new(
                action_key!("test.number_paginator"),
                "NumberPaginator",
                "Paginate numbers",
            ),
            total_pages: 100,
        },
    };
    let ctx = TestContextBuilder::minimal().build();
    let mut harness = StatefulTestHarness::new(action, ctx).unwrap();
    let input = serde_json::json!({});

    let r1 = harness.step(input.clone()).await.unwrap();
    assert!(r1.is_continue(), "page 1 of max 2 should Continue");

    let r2 = harness.step(input).await.unwrap();
    assert!(
        matches!(r2, ActionResult::Break { .. }),
        "page 2 should Break due to max_pages=2"
    );

    assert_eq!(harness.iterations(), 2);
}

#[tokio::test]
async fn paginated_single_page() {
    let action = NumberPaginator {
        meta: ActionMetadata::new(
            action_key!("test.number_paginator"),
            "NumberPaginator",
            "Paginate numbers",
        ),
        total_pages: 1,
    };
    let ctx = TestContextBuilder::minimal().build();
    let mut harness = StatefulTestHarness::new(action, ctx).unwrap();
    let input = serde_json::json!({});

    let r1 = harness.step(input).await.unwrap();
    assert!(
        matches!(r1, ActionResult::Break { .. }),
        "single page should Break immediately"
    );

    assert_eq!(harness.iterations(), 1);
}
