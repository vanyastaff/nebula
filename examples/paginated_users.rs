//! `PaginatedAction` DX trait demo — paginates dummyjson users by `skip` cursor.
//!
//! Run:
//!
//! ```bash
//! cargo run -p nebula-examples --bin paginated_users
//! ```
//!
//! The action walks <https://dummyjson.com/users> with cursor-based pagination
//! (`?limit=N&skip=S`) until the server signals no more pages. `main` drives
//! it through [`TestRuntime::run_stateful`] — one line for context, one for
//! the run, no adapter wiring.

use nebula_sdk::prelude::{
    Action, ActionContext, ActionError, ActionMetadata, DeclaresDependencies, Deserialize,
    PageResult, PaginatedAction, Serialize, TestContextBuilder, TestRuntime, Value, action_key,
    impl_paginated_action, json,
};

// ── Action definition ──────────────────────────────────────────────────────

struct DummyJsonUsersAction {
    meta: ActionMetadata,
}

impl DummyJsonUsersAction {
    fn new() -> Self {
        Self {
            meta: ActionMetadata::new(
                action_key!("dummyjson.users"),
                "DummyJSON Users",
                "Paginate https://dummyjson.com/users via PaginatedAction DX trait",
            ),
        }
    }
}

impl DeclaresDependencies for DummyJsonUsersAction {}
impl Action for DummyJsonUsersAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

#[derive(Deserialize)]
struct DummyJsonUsersResponse {
    users: Vec<DummyJsonUserRaw>,
    total: u32,
    skip: u32,
    limit: u32,
}

#[derive(Deserialize)]
struct DummyJsonUserRaw {
    id: u32,
    #[serde(rename = "firstName")]
    first_name: String,
    #[serde(rename = "lastName")]
    last_name: String,
    email: String,
    age: u32,
}

#[derive(Serialize)]
struct DummyJsonUserBrief {
    id: u32,
    name: String,
    email: String,
    age: u32,
}

impl From<DummyJsonUserRaw> for DummyJsonUserBrief {
    fn from(u: DummyJsonUserRaw) -> Self {
        Self {
            id: u.id,
            name: format!("{} {}", u.first_name, u.last_name),
            email: u.email,
            age: u.age,
        }
    }
}

impl PaginatedAction for DummyJsonUsersAction {
    type Input = Value;
    type Output = Vec<DummyJsonUserBrief>;
    type Cursor = u32;

    fn max_pages(&self) -> u32 {
        20
    }

    async fn fetch_page(
        &self,
        input: &Self::Input,
        cursor: Option<&u32>,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<PageResult<Vec<DummyJsonUserBrief>, u32>, ActionError> {
        let limit = input
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(10)
            .min(100) as u32;
        let skip = cursor.copied().unwrap_or(0);

        let url = format!("https://dummyjson.com/users?limit={limit}&skip={skip}");
        let response = reqwest::get(&url).await.map_err(|e| {
            if e.is_timeout() || e.is_connect() {
                ActionError::retryable(format!("fetch failed: {e}"))
            } else {
                ActionError::fatal(format!("fetch failed: {e}"))
            }
        })?;

        if !response.status().is_success() {
            return Err(ActionError::fatal(format!(
                "dummyjson returned HTTP {}",
                response.status()
            )));
        }

        let parsed: DummyJsonUsersResponse = response
            .json()
            .await
            .map_err(|e| ActionError::fatal(format!("parse failed: {e}")))?;

        let next_skip = parsed.skip.saturating_add(parsed.limit);
        let next_cursor = (next_skip < parsed.total).then_some(next_skip);
        let data = parsed.users.into_iter().map(Into::into).collect();

        Ok(PageResult { data, next_cursor })
    }
}

impl_paginated_action!(DummyJsonUsersAction);

// ── Runner ─────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("▶ paginated_users — fetching dummyjson.com/users with limit=30");
    println!();

    let ctx = TestContextBuilder::new().with_input(json!({ "limit": 30 }));
    let report = TestRuntime::new(ctx)
        .run_stateful(DummyJsonUsersAction::new())
        .await?;

    println!("kind:       {}", report.kind);
    println!("iterations: {}", report.iterations);
    println!("duration:   {:?}", report.duration);
    if let Some(note) = &report.note {
        println!("note:       {note}");
    }
    println!();
    println!("Final page output:");
    println!("{}", serde_json::to_string_pretty(&report.output)?);

    Ok(())
}
