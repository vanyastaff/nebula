//! Execution listing projection and history (migration 0064).
//!
//! Before 0064 the status column was written once and never followed a
//! commit, so nothing could filter on it; these assertions pin the contract
//! that replaced it, identically on every backend.

use nebula_storage_port::store::ExecutionStore;
use nebula_storage_port::{
    ExecutionHistoryCursor, ExecutionHistoryPageSize, ExecutionHistoryQuery, ExecutionListing,
    ExecutionListingStatus as Status, ExecutionStatusSet, FencingToken, MicrosInstant, Scope,
    TransitionBatch, TransitionOutcome,
};

use super::{Backend, scope_a, scope_b};

fn micros(seconds: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp_micros(1_759_665_600_000_000 + seconds * 1_000_000 + 123_456)
        .expect("fixed instant is in range")
}

/// Commit `listing` as the next snapshot of `id`, returning the new version.
async fn commit_listing(
    store: &dyn ExecutionStore,
    scope: &Scope,
    id: &str,
    expected_version: u64,
    fencing: FencingToken,
    listing: ExecutionListing,
) -> u64 {
    let batch = TransitionBatch::builder()
        .scope(scope.clone())
        .execution_id(id)
        .expected_version(expected_version)
        .fencing(fencing)
        .state(
            serde_json::json!({"status": listing.status().as_str()}),
            listing,
        )
        .build()
        .expect("batch");
    match store.commit(batch).await.expect("commit") {
        TransitionOutcome::Applied { new_version } => new_version,
        other => panic!("listing commit must apply, got {other:?}"),
    }
}

async fn lease(store: &dyn ExecutionStore, scope: &Scope, id: &str) -> FencingToken {
    store
        .acquire_lease(scope, id, "holder", std::time::Duration::from_secs(30))
        .await
        .expect("acquire_lease")
        .expect("lease is free")
}

fn whole_history() -> ExecutionHistoryQuery {
    ExecutionHistoryQuery::new().with_page_size(ExecutionHistoryPageSize::MAX)
}

/// The status column follows every commit, the projection timestamps
/// round-trip at microsecond precision, and a terminal execution leaves the
/// active scan.
pub(crate) async fn assert_status_projection_follows_commit(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let s = scope_a();
    store
        .create(&s, "exe_listing", "wf_listing", serde_json::json!({}))
        .await
        .expect("create");
    let created = store.get(&s, "exe_listing").await.expect("get").unwrap();
    assert_eq!(
        created.status,
        Status::Created,
        "[{}] new row",
        backend.name()
    );

    let token = lease(store.as_ref(), &s, "exe_listing").await;
    let running = ExecutionListing::new(Status::Running, Some(micros(1)), None);
    let version = commit_listing(
        store.as_ref(),
        &s,
        "exe_listing",
        created.version,
        token,
        running,
    )
    .await;
    assert_eq!(
        store.get(&s, "exe_listing").await.unwrap().unwrap().status,
        Status::Running,
        "[{}] status column must follow the commit",
        backend.name()
    );
    assert!(
        store
            .list_all_running()
            .await
            .expect("active scan")
            .iter()
            .any(|row| row.id == "exe_listing"),
        "[{}] a running execution is in the active scan",
        backend.name()
    );

    let failed = ExecutionListing::new(Status::Failed, Some(micros(1)), Some(micros(2)));
    commit_listing(store.as_ref(), &s, "exe_listing", version, token, failed).await;
    let page = store
        .list_history(&s, &whole_history())
        .await
        .expect("history");
    let summary = page
        .items
        .iter()
        .find(|item| item.id == "exe_listing")
        .unwrap_or_else(|| panic!("[{}] history lists the execution", backend.name()));
    assert_eq!(summary.status, Status::Failed, "[{}]", backend.name());
    assert_eq!(summary.workflow_id, "wf_listing");
    assert_eq!(
        summary.started_at,
        Some(MicrosInstant::floor(micros(1))),
        "[{}] started_at",
        backend.name()
    );
    assert_eq!(
        summary.finished_at,
        Some(MicrosInstant::floor(micros(2))),
        "[{}] finished_at",
        backend.name()
    );
    assert!(
        !store
            .list_all_running()
            .await
            .expect("active scan")
            .iter()
            .any(|row| row.id == "exe_listing"),
        "[{}] a terminal execution leaves the active scan",
        backend.name()
    );
}

/// History is newest first with an id tiebreak, pages concatenate to the
/// whole history without gaps or repeats, an execution created while paging
/// never shifts a later page, and every filter narrows exactly.
pub(crate) async fn assert_history_orders_filters_and_pages(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let s = scope_a();
    let ids = ["exe_h1", "exe_h2", "exe_h3", "exe_h4", "exe_h5"];
    for (index, id) in ids.iter().enumerate() {
        let workflow = if index % 2 == 0 { "wf_even" } else { "wf_odd" };
        store
            .create(&s, id, workflow, serde_json::json!({}))
            .await
            .expect("create");
    }

    let whole = store
        .list_history(&s, &whole_history())
        .await
        .expect("whole history");
    assert_eq!(
        whole.items.len(),
        ids.len(),
        "[{}] every row",
        backend.name()
    );
    assert!(whole.next_cursor.is_none(), "[{}] one page", backend.name());
    assert!(
        whole.items.windows(2).all(|pair| {
            (pair[0].created_at, pair[0].id.as_str()) > (pair[1].created_at, pair[1].id.as_str())
        }),
        "[{}] strictly newest first with id tiebreak",
        backend.name()
    );

    // Page through two at a time; create a newer execution after page one.
    let two = ExecutionHistoryPageSize::new(2).expect("page size");
    let mut paged = Vec::new();
    let mut query = ExecutionHistoryQuery::new().with_page_size(two);
    let mut inserted = false;
    loop {
        let page = store.list_history(&s, &query).await.expect("page");
        assert!(page.items.len() <= 2);
        paged.extend(page.items.iter().map(|item| item.id.clone()));
        if !inserted {
            store
                .create(&s, "exe_h_late", "wf_even", serde_json::json!({}))
                .await
                .expect("create while paging");
            inserted = true;
        }
        let Some(cursor) = page.next_cursor else {
            break;
        };
        query = ExecutionHistoryQuery::new()
            .with_page_size(two)
            .with_cursor(cursor);
    }
    let expected: Vec<String> = whole.items.iter().map(|item| item.id.clone()).collect();
    assert_eq!(
        paged,
        expected,
        "[{}] pages concatenate to the snapshot taken before paging",
        backend.name()
    );

    let even = store
        .list_history(&s, &whole_history().with_workflow("wf_even"))
        .await
        .expect("workflow filter");
    assert_eq!(
        even.items.len(),
        4,
        "[{}] workflow filter: three originals plus the late one",
        backend.name()
    );
    assert!(even.items.iter().all(|item| item.workflow_id == "wf_even"));

    let token = lease(store.as_ref(), &s, "exe_h2").await;
    commit_listing(
        store.as_ref(),
        &s,
        "exe_h2",
        0,
        token,
        ExecutionListing::new(Status::Failed, None, Some(micros(5))),
    )
    .await;
    let failed = store
        .list_history(
            &s,
            &whole_history().with_statuses([Status::Failed, Status::TimedOut].into()),
        )
        .await
        .expect("status filter");
    let failed_ids: Vec<&str> = failed.items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(failed_ids, ["exe_h2"], "[{}] status filter", backend.name());
    let active = store
        .list_history(
            &s,
            &whole_history().with_statuses(ExecutionStatusSet::ACTIVE),
        )
        .await
        .expect("active filter");
    assert_eq!(
        active.items.len(),
        ids.len(),
        "[{}] six rows minus the failed one",
        backend.name()
    );

    // Half-open creation range around the oldest row.
    let oldest = whole.items.last().expect("rows").clone();
    let from_oldest = store
        .list_history(
            &s,
            &whole_history().with_created_after(oldest.created_at.to_datetime()),
        )
        .await
        .expect("created_after");
    assert!(
        from_oldest.items.iter().any(|item| item.id == oldest.id),
        "[{}] created_after is inclusive",
        backend.name()
    );
    let before_oldest = store
        .list_history(
            &s,
            &whole_history().with_created_before(oldest.created_at.to_datetime()),
        )
        .await
        .expect("created_before");
    assert!(
        before_oldest
            .items
            .iter()
            .all(|item| item.created_at < oldest.created_at),
        "[{}] created_before is exclusive",
        backend.name()
    );
}

/// History never shows another tenant's executions, and a cursor built from
/// one tenant's rows pages only the caller's tenant.
pub(crate) async fn assert_history_is_scope_isolated(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    store
        .create(&scope_a(), "exe_iso_a", "wf_1", serde_json::json!({}))
        .await
        .expect("create a");
    store
        .create(&scope_b(), "exe_iso_b", "wf_1", serde_json::json!({}))
        .await
        .expect("create b");
    let b = store
        .list_history(&scope_b(), &whole_history())
        .await
        .expect("history b");
    let b_ids: Vec<&str> = b.items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(
        b_ids,
        ["exe_iso_b"],
        "[{}] tenant b sees only b",
        backend.name()
    );

    let a = store
        .list_history(&scope_a(), &whole_history())
        .await
        .expect("history a");
    let a_row = a
        .items
        .iter()
        .find(|item| item.id == "exe_iso_a")
        .expect("a row");
    let replayed = store
        .list_history(
            &scope_b(),
            &whole_history().with_cursor(ExecutionHistoryCursor::new(
                MicrosInstant::floor(a_row.created_at.to_datetime() + chrono::Duration::days(1)),
                "exe_zzz",
            )),
        )
        .await
        .expect("foreign cursor");
    assert!(
        replayed.items.iter().all(|item| item.id != "exe_iso_a"),
        "[{}] a cursor never crosses tenants",
        backend.name()
    );
}
